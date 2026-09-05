// crates/shell-web/src/routes/notification_settings.rs
//
// Per-user notification preferences. Available to every signed-in role (the
// backend `/v1/me/notification-preferences` family is self-service and authed).
// Renders three channel Switches — In-app, Email, Push — bound to
// get/set_notification_preferences, persisting on every toggle with a success
// toast (or a danger toast + local revert on failure).
//
// Follows the project-wide four-state UX: loading (skeleton), error, and loaded
// (there is no "empty" state — preferences always default to all-true).
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, PageHeader, SkeletonCard,
    Switch, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::PrefDto;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::{PrivacySettings, SecuritySettings};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// One labelled preference row: a title + helper text on the left, a Switch on
/// the right. Pure/presentational so it's SSR-testable.
fn pref_row(title: &str, help: &str, checked: bool, on_change: EventHandler<bool>) -> Element {
    rsx! {
        div { class: "notif-pref-row",
            div { class: "notif-pref-row-text",
                span { class: "notif-pref-row-title", "{title}" }
                span { class: "notif-pref-row-help", "{help}" }
            }
            Switch {
                checked,
                on_change: move |v| on_change.call(v),
            }
        }
    }
}

/// The "Browser notifications" affordance: a labelled row + an "Enable browser
/// notifications" button. Pure/presentational so it's SSR-testable; the click
/// handler is supplied by the caller (wasm wires it to the FCM bridge, SSR/test
/// passes a no-op).
fn browser_push_section(busy: bool, on_enable: EventHandler<()>) -> Element {
    let (title, help, button) = if cfg!(target_arch = "wasm32") {
        (
            "Browser notifications",
            "Get push notifications in this browser, even when AulaLite isn't open.",
            "Enable browser notifications",
        )
    } else {
        (
            "Device notifications",
            "Get push notifications on this device, even when AulaLite isn't open.",
            "Enable device notifications",
        )
    };
    rsx! {
        div { class: "notif-pref-row",
            div { class: "notif-pref-row-text",
                span { class: "notif-pref-row-title", "{title}" }
                span { class: "notif-pref-row-help",
                    "{help}"
                }
            }
            Button {
                label: button.to_string(),
                variant: ButtonVariant::Secondary,
                loading: busy,
                on_click: move |_| on_enable.call(()),
            }
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
enum BrowserPushStatus {
    Idle,
    Enabled,
    MissingConfig,
    Unsupported,
    PermissionDenied,
    ServiceWorkerFailed,
    TokenFailed,
    #[cfg(not(target_arch = "wasm32"))]
    AwaitingNativeToken,
    #[cfg(not(target_arch = "wasm32"))]
    RotationPending,
}

fn push_status_copy(status: &BrowserPushStatus) -> &'static str {
    match status {
        BrowserPushStatus::Idle => "",
        BrowserPushStatus::Enabled => "Push notifications are enabled on this device.",
        BrowserPushStatus::MissingConfig => "Browser push is not configured for this workspace.",
        BrowserPushStatus::Unsupported => "Push notifications are not supported on this device.",
        BrowserPushStatus::PermissionDenied => "Notifications are blocked in this browser.",
        BrowserPushStatus::ServiceWorkerFailed => {
            "The notification service worker could not start."
        }
        BrowserPushStatus::TokenFailed => "The browser could not create a push token.",
        #[cfg(not(target_arch = "wasm32"))]
        BrowserPushStatus::AwaitingNativeToken => {
            "The device notification service is still registering. Try again in a moment."
        }
        #[cfg(not(target_arch = "wasm32"))]
        BrowserPushStatus::RotationPending => {
            "This device is enabled, but an older notification token still needs revocation. Try Enable again when you are online."
        }
    }
}

fn device_label(device: &api::DeviceTokenDto) -> String {
    device
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(&device.platform)
        .to_string()
}

#[cfg(target_arch = "wasm32")]
fn browser_label_from_user_agent(user_agent: &str) -> &'static str {
    if user_agent.contains("Edg/") {
        "Edge"
    } else if user_agent.contains("Firefox") {
        "Firefox"
    } else if user_agent.contains("Chrome") || user_agent.contains("Chromium") {
        "Chrome"
    } else if user_agent.contains("Safari") {
        "Safari"
    } else {
        "Browser"
    }
}

fn device_list_section(
    devices: &[api::DeviceTokenDto],
    error: Option<&str>,
    loading: bool,
    on_retry: EventHandler<()>,
    on_revoke: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "notif-device-section",
            div { class: "notif-device-header",
                div { class: "notif-device-heading",
                    span { class: "notif-device-title", "Registered devices" }
                    span { class: "notif-device-help",
                        "Browsers and devices allowed to receive push notifications."
                    }
                }
                Button {
                    label: "Refresh".to_string(),
                    variant: ButtonVariant::Secondary,
                    loading,
                    on_click: move |_| on_retry.call(()),
                }
            }
            if let Some(error) = error {
                p { class: "notif-device-error", "Could not load devices: {error}" }
            }
            if loading && devices.is_empty() {
                p { class: "notif-device-loading", "Loading registered devices..." }
            } else if devices.is_empty() {
                p { class: "notif-device-empty", "No registered devices yet." }
            } else {
                ul { class: "notif-device-list",
                    for device in devices.iter() {
                        {
                            let id = device.id.clone();
                            let label = device_label(device);
                            let user_agent = device
                                .user_agent
                                .as_deref()
                                .map(str::trim)
                                .filter(|ua| !ua.is_empty())
                                .unwrap_or("Unknown browser")
                                .to_string();
                            let revoke = on_revoke;
                            rsx! {
                                li { class: "notif-device-item", key: "{id}",
                                    div { class: "notif-device-main",
                                        div { class: "notif-device-name-row",
                                            span { class: "notif-device-label", "{label}" }
                                            Badge {
                                                label: device.platform.clone(),
                                                tone: BadgeTone::Info,
                                            }
                                        }
                                        span { class: "notif-device-meta",
                                            "{device.platform} - Last seen {device.last_seen_at}"
                                        }
                                        span { class: "notif-device-user-agent", "{user_agent}" }
                                    }
                                    Button {
                                        label: "Revoke".to_string(),
                                        variant: ButtonVariant::Danger,
                                        on_click: move |_| revoke.call(id.clone()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn NotificationSettings() -> Element {
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

    // Local editable state: None until the initial fetch resolves.
    let mut prefs = use_signal(|| None::<PrefDto>);
    let mut load_error = use_signal(|| None::<String>);
    let saving = use_signal(|| false);

    // Initial load.
    use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                match api::get_notification_preferences(&api).await {
                    Ok(p) => prefs.set(Some(p)),
                    Err(e) => load_error.set(Some(format!("{e}"))),
                }
            }
        }
    });

    // Persist the full preference triple, optimistically updating local state
    // and reverting on failure.
    let save = {
        let api = api.clone();
        move |next: PrefDto| {
            let api = api.clone();
            let mut prefs = prefs;
            let mut saving = saving;
            let mut toast = toast;
            let prev = prefs.read().clone();
            prefs.set(Some(next.clone()));
            saving.set(true);
            spawn(async move {
                match api::set_notification_preferences(
                    &api,
                    next.email_enabled,
                    next.push_enabled,
                    next.in_app_enabled,
                )
                .await
                {
                    Ok(saved) => {
                        prefs.set(Some(saved));
                        toast.push(
                            ToastLevel::Success,
                            "Preferences saved",
                            "Your notification settings have been updated.",
                        );
                    }
                    Err(err) => {
                        // Revert to the pre-edit state so the UI stays truthful.
                        prefs.set(prev);
                        toast.push(ToastLevel::Danger, "Could not save", format!("{err}"));
                    }
                }
                saving.set(false);
            });
        }
    };

    // Leaderboard visibility (gamification opt-out, tenant-wide). Loaded
    // separately from notification prefs; saved on toggle with optimistic
    // update + revert-on-failure, mirroring the channel switches.
    let mut lb_visible = use_signal(|| None::<bool>);
    let mut lb_error = use_signal(|| None::<String>);
    use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                match api::get_leaderboard_opt_out(&api).await {
                    Ok(o) => lb_visible.set(Some(!o.opted_out)),
                    Err(e) => lb_error.set(Some(format!("{e}"))),
                }
            }
        }
    });
    let save_lb = {
        let api = api.clone();
        move |visible: bool| {
            let api = api.clone();
            let mut lb_visible = lb_visible;
            let mut toast = toast;
            let prev = *lb_visible.read();
            lb_visible.set(Some(visible));
            spawn(async move {
                match api::put_leaderboard_opt_out(&api, !visible).await {
                    Ok(saved) => {
                        lb_visible.set(Some(!saved.opted_out));
                        toast.push(
                            ToastLevel::Success,
                            "Preference saved",
                            if visible {
                                "You will appear on course leaderboards."
                            } else {
                                "You are now hidden from all leaderboards."
                            },
                        );
                    }
                    Err(err) => {
                        lb_visible.set(prev);
                        toast.push(ToastLevel::Danger, "Could not save", format!("{err}"));
                    }
                }
            });
        }
    };

    // Browser-push enrollment state + handler. On wasm, clicking "Enable
    // browser notifications" drives the FCM JS bridge (register SW + request
    // permission + fetch token) and registers the token with the backend.
    let push_busy = use_signal(|| false);
    let devices = use_signal(Vec::<api::DeviceTokenDto>::new);
    let devices_loading = use_signal(|| true);
    let devices_error = use_signal(|| None::<String>);
    let push_status = use_signal(|| BrowserPushStatus::Idle);

    let load_devices = {
        let api = api.clone();
        move || {
            let api = api.clone();
            let mut devices = devices;
            let mut devices_loading = devices_loading;
            let mut devices_error = devices_error;
            spawn(async move {
                devices_loading.set(true);
                devices_error.set(None);
                match api::list_device_tokens(&api).await {
                    Ok(resp) => devices.set(resp.devices),
                    Err(e) => devices_error.set(Some(format!("{e}"))),
                }
                devices_loading.set(false);
            });
        }
    };

    {
        let load_devices = load_devices.clone();
        use_effect(move || {
            load_devices();
        });
    }

    let revoke_device = {
        let api = api.clone();
        move |id: String| {
            let api = api.clone();
            let mut devices = devices;
            let mut toast = toast;
            spawn(async move {
                match api::revoke_device_token(&api, &id).await {
                    Ok(()) => {
                        devices.write().retain(|device| device.id != id);
                        toast.push(
                            ToastLevel::Success,
                            "Device revoked",
                            "This device will no longer receive push notifications.",
                        );
                    }
                    Err(err) => toast.push(
                        ToastLevel::Danger,
                        "Could not revoke device",
                        format!("{err}"),
                    ),
                }
            });
        }
    };

    let enable_browser_push = {
        let api = api.clone();
        let load_devices = load_devices.clone();
        move |_: ()| {
            let _api = api.clone();
            let load_devices = load_devices.clone();
            let mut push_busy = push_busy;
            let mut push_status = push_status;
            let mut toast = toast;
            #[cfg(target_arch = "wasm32")]
            {
                if *push_busy.read() {
                    return;
                }
                push_busy.set(true);
                spawn(async move {
                    match platform_bridge::web::fcm_request_token_outcome().await {
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::Token(token)) => {
                            let user_agent = web_sys::window()
                                .and_then(|window| window.navigator().user_agent().ok());
                            let label = user_agent
                                .as_deref()
                                .map(browser_label_from_user_agent)
                                .unwrap_or("Browser")
                                .to_string();
                            // Permission granted (or already granted) and a token
                            // was issued — (re)register it with the backend.
                            match api::register_device_token_with_metadata(
                                &_api,
                                &token,
                                "web",
                                Some(&label),
                                user_agent.as_deref(),
                            )
                            .await
                            {
                                Ok(()) => {
                                    push_status.set(BrowserPushStatus::Enabled);
                                    load_devices();
                                    toast.push(
                                        ToastLevel::Success,
                                        "Browser notifications enabled",
                                        "This browser will now receive push notifications.",
                                    );
                                }
                                Err(err) => toast.push(
                                    ToastLevel::Danger,
                                    "Could not enable",
                                    format!("{err}"),
                                ),
                            }
                        }
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::MissingVapidKey) => {
                            push_status.set(BrowserPushStatus::MissingConfig);
                            toast.push(
                                ToastLevel::Info,
                                "Push isn't configured yet",
                                push_status_copy(&BrowserPushStatus::MissingConfig),
                            );
                        }
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::Unsupported) => {
                            push_status.set(BrowserPushStatus::Unsupported);
                            toast.push(
                                ToastLevel::Info,
                                "Browser push unavailable",
                                push_status_copy(&BrowserPushStatus::Unsupported),
                            );
                        }
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::PermissionDenied) => {
                            push_status.set(BrowserPushStatus::PermissionDenied);
                            toast.push(
                                ToastLevel::Info,
                                "Notifications blocked",
                                push_status_copy(&BrowserPushStatus::PermissionDenied),
                            );
                        }
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::ServiceWorkerFailed) => {
                            push_status.set(BrowserPushStatus::ServiceWorkerFailed);
                            toast.push(
                                ToastLevel::Danger,
                                "Could not enable",
                                push_status_copy(&BrowserPushStatus::ServiceWorkerFailed),
                            );
                        }
                        Ok(platform_bridge::web::FcmRequestTokenOutcome::TokenFailed) => {
                            push_status.set(BrowserPushStatus::TokenFailed);
                            toast.push(
                                ToastLevel::Danger,
                                "Could not enable",
                                push_status_copy(&BrowserPushStatus::TokenFailed),
                            );
                        }
                        Err(err) => {
                            push_status.set(BrowserPushStatus::TokenFailed);
                            toast.push(ToastLevel::Danger, "Could not enable", format!("{err}"));
                        }
                    }
                    push_busy.set(false);
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if *push_busy.read() {
                    return;
                }
                push_busy.set(true);
                spawn(async move {
                    if platform_bridge::native_push::native_push_platform() == Some("android") {
                        if let Err(error) =
                            platform_bridge::native_push::request_permission_from_host()
                        {
                            push_status.set(BrowserPushStatus::TokenFailed);
                            toast.push(ToastLevel::Danger, "Could not enable", format!("{error}"));
                            push_busy.set(false);
                            return;
                        }
                        // Token refresh is asynchronous on the Android host.
                        // Give the already-initialized Firebase task a brief
                        // opportunity to commit its protected registration.
                        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    }
                    match platform_bridge::native_push::registration_token_outcome() {
                        Ok(platform_bridge::native_push::NativePushTokenOutcome::Token(
                            registration,
                        )) => {
                            match api::register_device_token_with_metadata(
                                &_api,
                                &registration.token,
                                &registration.platform,
                                Some(&registration.label),
                                None,
                            )
                            .await
                            {
                                Ok(()) => {
                                    load_devices();
                                    let old_token_revoked = if let Some(previous) =
                                        registration.previous_token.as_deref()
                                    {
                                        api::remove_device_token(&_api, previous).await.is_ok()
                                    } else {
                                        true
                                    };
                                    if old_token_revoked {
                                        match platform_bridge::native_push::mark_registration_synced()
                                        {
                                            Ok(()) => {
                                                push_status.set(BrowserPushStatus::Enabled);
                                                toast.push(
                                                    ToastLevel::Success,
                                                    "Device notifications enabled",
                                                    "This device will now receive push notifications.",
                                                );
                                            }
                                            Err(error) => {
                                                push_status.set(BrowserPushStatus::TokenFailed);
                                                toast.push(
                                                    ToastLevel::Danger,
                                                    "Could not save device state",
                                                    format!("{error}"),
                                                );
                                            }
                                        }
                                    } else {
                                        // Keep `previous_token` in the protected store. The next
                                        // Enable attempt retries the idempotent backend revoke.
                                        push_status.set(BrowserPushStatus::RotationPending);
                                        toast.push(
                                            ToastLevel::Info,
                                            "Older device token still active",
                                            push_status_copy(&BrowserPushStatus::RotationPending),
                                        );
                                    }
                                }
                                Err(error) => {
                                    push_status.set(BrowserPushStatus::TokenFailed);
                                    toast.push(
                                        ToastLevel::Danger,
                                        "Could not enable",
                                        format!("{error}"),
                                    );
                                }
                            }
                        }
                        Ok(
                            platform_bridge::native_push::NativePushTokenOutcome::AwaitingHostRegistration,
                        ) => {
                            push_status.set(BrowserPushStatus::AwaitingNativeToken);
                            toast.push(
                                ToastLevel::Info,
                                "Device still registering",
                                push_status_copy(&BrowserPushStatus::AwaitingNativeToken),
                            );
                        }
                        Ok(platform_bridge::native_push::NativePushTokenOutcome::Unsupported) => {
                            push_status.set(BrowserPushStatus::Unsupported);
                            toast.push(
                                ToastLevel::Info,
                                "Push unavailable",
                                push_status_copy(&BrowserPushStatus::Unsupported),
                            );
                        }
                        Err(error) => {
                            push_status.set(BrowserPushStatus::TokenFailed);
                            toast.push(ToastLevel::Danger, "Could not enable", format!("{error}"));
                        }
                    }
                    push_busy.set(false);
                });
            }
        }
    };

    let content: Element = match (prefs.read().clone(), load_error.read().clone()) {
        (Some(p), _) => {
            let in_app = p.in_app_enabled;
            let email = p.email_enabled;
            let push = p.push_enabled;

            let save_in_app = save.clone();
            let save_email = save.clone();
            let save_push = save.clone();
            let base = p.clone();
            let base_email = p.clone();
            let base_push = p.clone();
            let enable_push = enable_browser_push.clone();
            let push_is_busy = *push_busy.read();
            let push_status_text = {
                let status = push_status.read();
                push_status_copy(&status)
            };
            let device_rows = devices.read().clone();
            let device_error = devices_error.read().clone();
            let device_loading = *devices_loading.read();
            let retry_devices = load_devices.clone();
            let revoke_push_device = revoke_device.clone();

            rsx! {
                Card {
                    div { class: "notif-pref-list",
                        {pref_row(
                            "In-app notifications",
                            "Show notifications in the bell menu.",
                            in_app,
                            EventHandler::new(move |v: bool| {
                                save_in_app(PrefDto { in_app_enabled: v, ..base.clone() });
                            }),
                        )}
                        {pref_row(
                            "Email notifications",
                            "Send important updates to your email.",
                            email,
                            EventHandler::new(move |v: bool| {
                                save_email(PrefDto { email_enabled: v, ..base_email.clone() });
                            }),
                        )}
                        {pref_row(
                            "Push notifications",
                            "Send push notifications to your devices.",
                            push,
                            EventHandler::new(move |v: bool| {
                                save_push(PrefDto { push_enabled: v, ..base_push.clone() });
                            }),
                        )}
                        {browser_push_section(
                            push_is_busy,
                            EventHandler::new(move |_| enable_push(())),
                        )}
                        if !push_status_text.is_empty() {
                            p { class: "notif-push-status", "{push_status_text}" }
                        }
                    }
                }
                Card {
                    {device_list_section(
                        &device_rows,
                        device_error.as_deref(),
                        device_loading,
                        EventHandler::new(move |_| retry_devices()),
                        EventHandler::new(move |id: String| revoke_push_device(id)),
                    )}
                }
            }
        }
        (None, Some(e)) => rsx! {
            p { class: "error", "Could not load notification preferences: {e}" }
        },
        (None, None) => rsx! {
            SkeletonCard { height: "200px".to_string() }
        },
    };

    // Privacy section: leaderboard visibility. Hidden entirely while loading
    // or on error — it's a secondary control with its own home on the
    // leaderboard tab, so a failure here shouldn't degrade the page.
    let privacy_section: Element = match (*lb_visible.read(), lb_error.read().clone()) {
        (Some(visible), _) => {
            let save_lb = save_lb.clone();
            rsx! {
                h2 { class: "notif-settings-section-title", "Privacy" }
                Card {
                    div { class: "notif-pref-list",
                        {pref_row(
                            "Show me on leaderboards",
                            "Appear in course leaderboards. Turning this off hides you across all courses.",
                            visible,
                            EventHandler::new(move |v: bool| save_lb(v)),
                        )}
                    }
                }
            }
        }
        _ => rsx! {},
    };

    let api_for_privacy_signout = api.clone();
    let body = rsx! {
        div { class: "notif-settings-page",
            PageHeader {
                title: "Notifications".to_string(),
                kicker: "Settings".to_string(),
                subtitle: "Choose how you'd like to be notified.".to_string(),
            }
            { content }
            { privacy_section }
            SecuritySettings {}
            PrivacySettings {
                on_signed_out: move |_| {
                    let cleanup_api = api_for_privacy_signout.clone();
                    spawn(async move { crate::routes::sign_out_with_device_cleanup(cleanup_api).await; });
                    nav.push(Route::Login {});
                },
            }
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    let api_for_shell_signout = api.clone();
    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                let cleanup_api = api_for_shell_signout.clone();
                spawn(async move { crate::routes::sign_out_with_device_cleanup(cleanup_api).await; });
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
    fn pref_row_renders_title_help_and_switch() {
        fn app() -> Element {
            pref_row(
                "Email notifications",
                "Send important updates to your email.",
                true,
                EventHandler::new(|_| {}),
            )
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Email notifications"),
            "title missing: {html}"
        );
        assert!(
            html.contains("Send important updates"),
            "help text missing: {html}"
        );
        // The Switch primitive renders with the ds-switch class and reflects
        // the checked state.
        assert!(html.contains("ds-switch"), "switch missing: {html}");
        assert!(html.contains("checked"), "checked state missing: {html}");
    }

    #[test]
    fn three_pref_rows_render_all_channels() {
        // Render the loaded body shape (all three channel rows) the way the
        // page composes them, without needing a live ApiContext.
        fn app() -> Element {
            let p = PrefDto {
                email_enabled: true,
                push_enabled: false,
                in_app_enabled: true,
            };
            rsx! {
                div {
                    {pref_row("In-app notifications", "Show notifications in the bell menu.", p.in_app_enabled, EventHandler::new(|_| {}))}
                    {pref_row("Email notifications", "Send important updates to your email.", p.email_enabled, EventHandler::new(|_| {}))}
                    {pref_row("Push notifications", "Send push notifications to your devices.", p.push_enabled, EventHandler::new(|_| {}))}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("In-app notifications"));
        assert!(html.contains("Email notifications"));
        assert!(html.contains("Push notifications"));
        // Three switches rendered.
        assert_eq!(html.matches("ds-switch-input").count(), 3, "got: {html}");
    }

    #[test]
    fn browser_push_section_renders_enable_affordance() {
        // The renderer-appropriate enrollment button remains discoverable.
        fn app() -> Element {
            browser_push_section(false, EventHandler::new(|_| {}))
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        let expected_button = if cfg!(target_arch = "wasm32") {
            "Enable browser notifications"
        } else {
            "Enable device notifications"
        };
        let expected_title = if cfg!(target_arch = "wasm32") {
            "Browser notifications"
        } else {
            "Device notifications"
        };
        assert!(
            html.contains(expected_button),
            "enable button label missing: {html}"
        );
        assert!(
            html.contains(expected_title),
            "section title missing: {html}"
        );
        // Rendered as a design-system button.
        assert!(html.contains("ds-button"), "button missing: {html}");
    }

    #[test]
    fn device_table_renders_registered_devices_without_tokens() {
        fn app() -> Element {
            device_list_section(
                &[api::DeviceTokenDto {
                    id: "device-1".into(),
                    platform: "web".into(),
                    label: Some("Chrome".into()),
                    user_agent: Some("Mozilla/5.0".into()),
                    created_at: "2026-06-18T00:00:00Z".into(),
                    last_seen_at: "2026-06-18T00:01:00Z".into(),
                }],
                None,
                false,
                EventHandler::new(|_| {}),
                EventHandler::new(|_| {}),
            )
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Registered devices"), "{html}");
        assert!(html.contains("Chrome"), "{html}");
        assert!(html.contains("Revoke"), "{html}");
        assert!(!html.contains("tok-"), "{html}");
    }

    #[test]
    fn browser_push_status_text_distinguishes_denied_permission() {
        assert_eq!(
            push_status_copy(&BrowserPushStatus::PermissionDenied),
            "Notifications are blocked in this browser."
        );
        assert_eq!(
            push_status_copy(&BrowserPushStatus::MissingConfig),
            "Browser push is not configured for this workspace."
        );
    }
}
