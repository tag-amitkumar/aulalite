// crates/shell-web/src/routes/admin_integrations.rs
//
// Admin integrations console. Tenant-scoped integration setup for API keys,
// outbound webhooks, SSO, and LTI platforms.
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Input, PageHeader, Sheet, SheetBody,
    SheetClose, SheetHeader, SheetTitle, SkeletonCard, Switch, Tab, Table, Tabs, TabsVariant,
    ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::{
    AdminApiKeyDto, AdminLtiPlatformDto, AdminSsoConfigDto, AdminWebhookDeliveryDto,
    AdminWebhookSubscriptionDto, RegisterAdminLtiPlatformBody, UpdateAdminWebhookSubscriptionBody,
    UpsertAdminSsoConfigBody,
};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

const WEBHOOK_DELIVERY_LIMIT: i64 = 50;

fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

fn blank(raw: Option<&str>) -> String {
    raw.filter(|s| !s.trim().is_empty())
        .unwrap_or("Never")
        .to_string()
}

fn active_badge(active: bool) -> Element {
    if active {
        rsx! { Badge { label: "Active".to_string(), tone: BadgeTone::Success } }
    } else {
        rsx! { Badge { label: "Inactive".to_string(), tone: BadgeTone::Neutral } }
    }
}

fn key_badge(key: &AdminApiKeyDto) -> Element {
    if key.revoked_at.is_some() {
        rsx! { Badge { label: "Revoked".to_string(), tone: BadgeTone::Danger } }
    } else {
        rsx! { Badge { label: "Active".to_string(), tone: BadgeTone::Success } }
    }
}

fn delivery_tone(status: &str) -> BadgeTone {
    match status {
        "sent" | "delivered" | "success" => BadgeTone::Success,
        "failed" | "error" => BadgeTone::Danger,
        "queued" | "pending" | "retrying" => BadgeTone::Warning,
        _ => BadgeTone::Neutral,
    }
}

fn scopes_text(scopes: &[String]) -> String {
    if scopes.is_empty() {
        "read".to_string()
    } else {
        scopes.join(", ")
    }
}

fn payload_preview(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn matches_any(haystacks: &[&str], query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    haystacks
        .iter()
        .any(|h| h.to_ascii_lowercase().contains(&q))
}

#[cfg(target_arch = "wasm32")]
fn confirm_action(message: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(message).ok())
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn confirm_action(_message: &str) -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IntegrationCounts {
    active_keys: usize,
    webhook_count: usize,
    sso_enabled: bool,
    lti_count: usize,
}

fn summary_grid(counts: IntegrationCounts) -> Element {
    let sso_state = if counts.sso_enabled {
        "Enabled"
    } else {
        "Not enabled"
    };
    rsx! {
        div { class: "integration-summary-grid",
            div { class: "integration-summary-tile",
                span { class: "integration-summary-label", "Active API keys" }
                strong { "{counts.active_keys}" }
            }
            div { class: "integration-summary-tile",
                span { class: "integration-summary-label", "Webhook endpoints" }
                strong { "{counts.webhook_count}" }
            }
            div { class: "integration-summary-tile",
                span { class: "integration-summary-label", "SSO" }
                strong { "{sso_state}" }
            }
            div { class: "integration-summary-tile",
                span { class: "integration-summary-label", "LTI platforms" }
                strong { "{counts.lti_count}" }
            }
        }
    }
}

fn secret_panel(title: &str, value: &str) -> Element {
    rsx! {
        div { class: "integration-secret-panel",
            p { class: "integration-secret-title", "{title}" }
            code { class: "integration-secret-value", "{value}" }
        }
    }
}

fn api_key_table(keys: &[AdminApiKeyDto], on_revoke: EventHandler<String>) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "Name" }
                    th { class: "ds-table-th", "Prefix" }
                    th { class: "ds-table-th", "Scopes" }
                    th { class: "ds-table-th", "Status" }
                    th { class: "ds-table-th", "Last used" }
                    th { class: "ds-table-th", "Created" }
                    th { class: "ds-table-th", "Action" }
                }
            },
            body: rsx! {
                for key in keys.iter() {
                    {
                        let id = key.id.clone();
                        let revoked = key.revoked_at.is_some();
                        let last_used = blank(key.last_used_at.as_deref());
                        let created = format_ts(&key.created_at);
                        let scopes = scopes_text(&key.scopes);
                        rsx! {
                            tr { key: "{key.id}",
                                td { "{key.name}" }
                                td { code { "{key.prefix}" } }
                                td { "{scopes}" }
                                td { {key_badge(key)} }
                                td { "{last_used}" }
                                td { "{created}" }
                                td {
                                    button {
                                        class: "linkish",
                                        disabled: revoked,
                                        onclick: move |_| on_revoke.call(id.clone()),
                                        "Revoke"
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn webhook_table(
    hooks: &[AdminWebhookSubscriptionDto],
    on_edit: EventHandler<AdminWebhookSubscriptionDto>,
    on_delete: EventHandler<String>,
) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "URL" }
                    th { class: "ds-table-th", "Events" }
                    th { class: "ds-table-th", "State" }
                    th { class: "ds-table-th", "Created" }
                    th { class: "ds-table-th", "Actions" }
                }
            },
            body: rsx! {
                for hook in hooks.iter() {
                    {
                        let row = hook.clone();
                        let row_for_edit = row.clone();
                        let id = hook.id.clone();
                        let created = format_ts(&hook.created_at);
                        let events = hook.events.join(", ");
                        rsx! {
                            tr { key: "{hook.id}",
                                td { code { "{hook.url}" } }
                                td { "{events}" }
                                td { {active_badge(hook.active)} }
                                td { "{created}" }
                                td { class: "integration-action-cell",
                                    button {
                                        class: "linkish",
                                        onclick: move |_| on_edit.call(row_for_edit.clone()),
                                        "Edit"
                                    }
                                    button {
                                        class: "linkish linkish-danger",
                                        onclick: move |_| on_delete.call(id.clone()),
                                        "Delete"
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn webhook_delivery_table(
    deliveries: &[AdminWebhookDeliveryDto],
    on_select: EventHandler<AdminWebhookDeliveryDto>,
) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "Created" }
                    th { class: "ds-table-th", "Event" }
                    th { class: "ds-table-th", "Status" }
                    th { class: "ds-table-th", "Attempts" }
                    th { class: "ds-table-th", "Response" }
                    th { class: "ds-table-th", "Details" }
                }
            },
            body: rsx! {
                for row in deliveries.iter() {
                    {
                        let delivery = row.clone();
                        let created = format_ts(&row.created_at);
                        let response = row.response_code.map(|c| c.to_string()).unwrap_or_else(|| "-".to_string());
                        rsx! {
                            tr { key: "{row.id}",
                                td { "{created}" }
                                td { "{row.event}" }
                                td { Badge { label: row.status.clone(), tone: delivery_tone(&row.status) } }
                                td { "{row.attempts}" }
                                td { "{response}" }
                                td {
                                    button {
                                        class: "linkish",
                                        onclick: move |_| on_select.call(delivery.clone()),
                                        "Open"
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn delivery_detail(open: Signal<bool>, delivery: Option<AdminWebhookDeliveryDto>) -> Element {
    rsx! {
        Sheet { open, width: 560, aria_label: "Webhook delivery details",
            SheetHeader {
                SheetTitle { "Webhook delivery" }
                SheetClose { open }
            }
            SheetBody {
                if let Some(row) = delivery {
                    {
                        let last_attempt = blank(row.last_attempt_at.as_deref());
                        let response = row.response_code.map(|c| c.to_string()).unwrap_or_else(|| "None".to_string());
                        let payload = payload_preview(&row.payload_json);
                        rsx! {
                            dl { class: "delivery-detail-list",
                                dt { "Event" } dd { "{row.event}" }
                                dt { "Status" } dd { "{row.status}" }
                                dt { "Attempts" } dd { "{row.attempts}" }
                                dt { "Last attempt" } dd { "{last_attempt}" }
                                dt { "Response" } dd { "{response}" }
                            }
                            pre { class: "integration-payload-preview", "{payload}" }
                        }
                    }
                }
            }
        }
    }
}

fn lti_table(platforms: &[AdminLtiPlatformDto], on_delete: EventHandler<String>) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "Name" }
                    th { class: "ds-table-th", "Issuer" }
                    th { class: "ds-table-th", "Client ID" }
                    th { class: "ds-table-th", "Deployment" }
                    th { class: "ds-table-th", "Created" }
                    th { class: "ds-table-th", "Action" }
                }
            },
            body: rsx! {
                for platform in platforms.iter() {
                    {
                        let id = platform.id.clone();
                        let created = format_ts(&platform.created_at);
                        rsx! {
                            tr { key: "{platform.id}",
                                td { "{platform.name}" }
                                td { code { "{platform.issuer}" } }
                                td { code { "{platform.client_id}" } }
                                td { "{platform.deployment_id}" }
                                td { "{created}" }
                                td {
                                    button {
                                        class: "linkish linkish-danger",
                                        onclick: move |_| on_delete.call(id.clone()),
                                        "Delete"
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn scope_checkbox(
    scope: &'static str,
    mut selected_scopes: Signal<Vec<String>>,
    disabled: bool,
) -> Element {
    let value = scope.to_string();
    let checked = selected_scopes.read().iter().any(|s| s == &value);
    rsx! {
        label { class: "integration-check",
            input {
                r#type: "checkbox",
                checked,
                disabled,
                onchange: move |evt| {
                    let mut next = selected_scopes.read().clone();
                    if evt.checked() {
                        if !next.iter().any(|s| s == scope) {
                            next.push(scope.to_string());
                        }
                    } else {
                        next.retain(|s| s != scope);
                    }
                    selected_scopes.set(next);
                },
            }
            code { "{scope}" }
        }
    }
}

fn event_checkbox(
    event: &'static str,
    mut selected_events: Signal<Vec<String>>,
    disabled: bool,
) -> Element {
    let value = event.to_string();
    let checked = selected_events.read().iter().any(|s| s == &value);
    rsx! {
        label { class: "integration-check",
            input {
                r#type: "checkbox",
                checked,
                disabled,
                onchange: move |evt| {
                    let mut next = selected_events.read().clone();
                    if evt.checked() {
                        if !next.iter().any(|s| s == event) {
                            next.push(event.to_string());
                        }
                    } else {
                        next.retain(|s| s != event);
                    }
                    selected_events.set(next);
                },
            }
            span { "{event}" }
        }
    }
}

#[component]
pub fn AdminIntegrations() -> Element {
    let nav = use_navigator();
    let api_ctx = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting..." } };
        }
    };

    let is_admin = user.can_manage_integrations();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to manage integrations." }
            }
        };
    }

    let toast = use_toast_sender();
    let mut active_tab = use_signal(|| "api-keys".to_string());
    let mut search = use_signal(String::new);

    let mut api_keys = use_resource({
        let api_ctx = api_ctx.clone();
        move || {
            let api_ctx = api_ctx.clone();
            async move { api::list_admin_api_keys(&api_ctx).await }
        }
    });
    let mut webhooks = use_resource({
        let api_ctx = api_ctx.clone();
        move || {
            let api_ctx = api_ctx.clone();
            async move { api::list_admin_webhooks(&api_ctx).await }
        }
    });
    let webhook_deliveries = use_resource({
        let api_ctx = api_ctx.clone();
        move || {
            let api_ctx = api_ctx.clone();
            async move {
                api::list_admin_webhook_deliveries(&api_ctx, None, Some(WEBHOOK_DELIVERY_LIMIT))
                    .await
            }
        }
    });
    let mut sso_config = use_resource({
        let api_ctx = api_ctx.clone();
        move || {
            let api_ctx = api_ctx.clone();
            async move { api::get_admin_sso_config(&api_ctx).await }
        }
    });
    let mut lti_platforms = use_resource({
        let api_ctx = api_ctx.clone();
        move || {
            let api_ctx = api_ctx.clone();
            async move { api::list_admin_lti_platforms(&api_ctx).await }
        }
    });

    let mut key_name = use_signal(String::new);
    let mut key_scopes = use_signal(|| vec!["read".to_string()]);
    let mut key_saving = use_signal(|| false);
    let mut minted_key = use_signal(|| None::<String>);

    let mut webhook_edit_id = use_signal(|| None::<String>);
    let mut webhook_url = use_signal(String::new);
    let mut webhook_events = use_signal(|| vec!["course.published".to_string()]);
    let mut webhook_active = use_signal(|| true);
    let mut webhook_saving = use_signal(|| false);
    let mut webhook_secret = use_signal(|| None::<String>);
    let mut selected_delivery = use_signal(|| None::<AdminWebhookDeliveryDto>);
    let mut delivery_sheet_open = use_signal(|| false);

    let mut sso_issuer = use_signal(String::new);
    let mut sso_client_id = use_signal(String::new);
    let mut sso_secret = use_signal(String::new);
    let mut sso_authorize_url = use_signal(String::new);
    let mut sso_token_url = use_signal(String::new);
    let mut sso_jwks_url = use_signal(String::new);
    let mut sso_enabled = use_signal(|| false);
    let mut sso_saving = use_signal(|| false);
    let mut sso_loaded = use_signal(|| false);

    {
        let sso_config = sso_config;
        use_effect(move || {
            if *sso_loaded.read() {
                return;
            }
            let snap = sso_config.read_unchecked();
            let Some(Ok(Some(cfg))) = snap.as_ref() else {
                return;
            };
            sso_issuer.set(cfg.issuer.clone());
            sso_client_id.set(cfg.client_id.clone());
            sso_authorize_url.set(cfg.authorize_url.clone());
            sso_token_url.set(cfg.token_url.clone());
            sso_jwks_url.set(cfg.jwks_url.clone());
            sso_enabled.set(cfg.enabled);
            sso_loaded.set(true);
        });
    }

    let mut lti_name = use_signal(String::new);
    let mut lti_issuer = use_signal(String::new);
    let mut lti_client_id = use_signal(String::new);
    let mut lti_auth_login_url = use_signal(String::new);
    let mut lti_jwks_url = use_signal(String::new);
    let mut lti_deployment_id = use_signal(String::new);
    let mut lti_default_course_id = use_signal(String::new);
    let mut lti_saving = use_signal(|| false);

    let counts = {
        let keys_snap = api_keys.read_unchecked();
        let hooks_snap = webhooks.read_unchecked();
        let sso_snap = sso_config.read_unchecked();
        let lti_snap = lti_platforms.read_unchecked();
        IntegrationCounts {
            active_keys: keys_snap
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|rows| rows.iter().filter(|key| key.revoked_at.is_none()).count())
                .unwrap_or(0),
            webhook_count: hooks_snap
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|rows| rows.len())
                .unwrap_or(0),
            sso_enabled: sso_snap
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|cfg| cfg.as_ref())
                .map(|cfg| cfg.enabled)
                .unwrap_or(false),
            lti_count: lti_snap
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|rows| rows.len())
                .unwrap_or(0),
        }
    };

    let tabs = vec![
        Tab {
            key: "api-keys".to_string(),
            label: "API keys".to_string(),
            ..Default::default()
        },
        Tab {
            key: "webhooks".to_string(),
            label: "Webhooks".to_string(),
            ..Default::default()
        },
        Tab {
            key: "sso".to_string(),
            label: "SSO".to_string(),
            ..Default::default()
        },
        Tab {
            key: "lti".to_string(),
            label: "LTI".to_string(),
            ..Default::default()
        },
    ];

    let current_query = search.read().trim().to_string();
    let active = active_tab.read().clone();

    let api_key_section: Element = {
        let snap = api_keys.read_unchecked();
        let table = match snap.as_ref() {
            Some(Ok(rows)) => {
                let filtered: Vec<AdminApiKeyDto> = rows
                    .iter()
                    .filter(|key| {
                        matches_any(
                            &[&key.name, &key.prefix, &key.scopes.join(", ")],
                            &current_query,
                        )
                    })
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    rsx! { p { class: "muted", "No API keys match the current filter." } }
                } else {
                    let api_ctx = api_ctx.clone();
                    let mut api_keys = api_keys;
                    api_key_table(
                        &filtered,
                        EventHandler::new(move |id: String| {
                            if !confirm_action("Revoke this API key?") {
                                return;
                            }
                            let api_ctx = api_ctx.clone();
                            let mut toast = toast;
                            spawn(async move {
                                match api::revoke_admin_api_key(&api_ctx, &id).await {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "API key revoked",
                                            "The key can no longer authenticate.",
                                        );
                                        api_keys.restart();
                                    }
                                    Err(err) => {
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Revoke failed",
                                            format!("{err}"),
                                        );
                                    }
                                }
                            });
                        }),
                    )
                }
            }
            Some(Err(e)) => rsx! { p { class: "error", "Could not load API keys: {e}" } },
            None => rsx! { SkeletonCard { height: "180px".to_string() } },
        };
        let api_ctx_for_key_submit = api_ctx.clone();
        rsx! {
            section { class: "integration-section",
                div { class: "integration-grid",
                    form {
                        class: "integration-form",
                        onsubmit: move |evt| {
                            evt.prevent_default();
                            let api_ctx = api_ctx_for_key_submit.clone();
                            let mut toast = toast;
                            let name = key_name.read().trim().to_string();
                            let scopes = key_scopes.read().clone();
                            if name.is_empty() {
                                toast.push(ToastLevel::Warning, "Name required", "Enter a key name.");
                                return;
                            }
                            key_saving.set(true);
                            spawn(async move {
                                match api::mint_admin_api_key(&api_ctx, &name, &scopes).await {
                                    Ok(created) => {
                                        minted_key.set(Some(created.plaintext));
                                        key_name.set(String::new());
                                        key_scopes.set(vec!["read".to_string()]);
                                        toast.push(ToastLevel::Success, "API key created", "Copy the key now; it will not be shown again.");
                                        api_keys.restart();
                                    }
                                    Err(err) => toast.push(ToastLevel::Danger, "Create failed", format!("{err}")),
                                }
                                key_saving.set(false);
                            });
                        },
                        h2 { "Create API key" }
                        label { class: "integration-label", "Name" }
                        Input {
                            value: key_name.read().clone(),
                            placeholder: "Reporting export".to_string(),
                            disabled: *key_saving.read(),
                            on_input: move |value| key_name.set(value),
                        }
                        fieldset { class: "integration-check-grid",
                            legend { "Scopes" }
                            for scope in api::ADMIN_API_KEY_SCOPES {
                                {scope_checkbox(scope, key_scopes, *key_saving.read())}
                            }
                        }
                        if let Some(value) = minted_key.read().as_ref() {
                            {secret_panel("New API key", value)}
                        }
                        Button {
                            label: "Create key".to_string(),
                            button_type: "submit".to_string(),
                            loading: *key_saving.read(),
                            on_click: |_| {},
                        }
                    }
                    div { class: "integration-table-panel",
                        h2 { "API keys" }
                        {table}
                    }
                }
            }
        }
    };

    let webhook_section: Element = {
        let hooks_snap = webhooks.read_unchecked();
        let deliveries_snap = webhook_deliveries.read_unchecked();
        let hooks_table = match hooks_snap.as_ref() {
            Some(Ok(rows)) => {
                let filtered: Vec<AdminWebhookSubscriptionDto> = rows
                    .iter()
                    .filter(|hook| {
                        matches_any(
                            &[
                                &hook.url,
                                &hook.events.join(", "),
                                if hook.active { "active" } else { "inactive" },
                            ],
                            &current_query,
                        )
                    })
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    rsx! { p { class: "muted", "No webhooks match the current filter." } }
                } else {
                    let api_ctx_for_delete = api_ctx.clone();
                    let mut webhooks = webhooks;
                    let mut deliveries = webhook_deliveries;
                    webhook_table(
                        &filtered,
                        EventHandler::new(move |row: AdminWebhookSubscriptionDto| {
                            webhook_edit_id.set(Some(row.id.clone()));
                            webhook_url.set(row.url.clone());
                            webhook_events.set(row.events.clone());
                            webhook_active.set(row.active);
                        }),
                        EventHandler::new(move |id: String| {
                            if !confirm_action("Delete this webhook endpoint and its delivery log?")
                            {
                                return;
                            }
                            let api_ctx = api_ctx_for_delete.clone();
                            let mut toast = toast;
                            spawn(async move {
                                match api::delete_admin_webhook(&api_ctx, &id).await {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Webhook deleted",
                                            "The endpoint was removed.",
                                        );
                                        webhooks.restart();
                                        deliveries.restart();
                                    }
                                    Err(err) => toast.push(
                                        ToastLevel::Danger,
                                        "Delete failed",
                                        format!("{err}"),
                                    ),
                                }
                            });
                        }),
                    )
                }
            }
            Some(Err(e)) => rsx! { p { class: "error", "Could not load webhooks: {e}" } },
            None => rsx! { SkeletonCard { height: "180px".to_string() } },
        };
        let deliveries_table = match deliveries_snap.as_ref() {
            Some(Ok(rows)) if rows.is_empty() => {
                rsx! { p { class: "muted", "No webhook deliveries recorded yet." } }
            }
            Some(Ok(rows)) => {
                let filtered: Vec<AdminWebhookDeliveryDto> = rows
                    .iter()
                    .filter(|row| matches_any(&[&row.event, &row.status], &current_query))
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    rsx! { p { class: "muted", "No deliveries match the current filter." } }
                } else {
                    webhook_delivery_table(
                        &filtered,
                        EventHandler::new(move |row: AdminWebhookDeliveryDto| {
                            selected_delivery.set(Some(row));
                            delivery_sheet_open.set(true);
                        }),
                    )
                }
            }
            Some(Err(e)) => rsx! { p { class: "error", "Could not load webhook deliveries: {e}" } },
            None => rsx! { SkeletonCard { height: "180px".to_string() } },
        };
        let editing = webhook_edit_id.read().is_some();
        let api_ctx_for_webhook_submit = api_ctx.clone();
        rsx! {
            section { class: "integration-section",
                div { class: "integration-grid",
                    form {
                        class: "integration-form",
                        onsubmit: move |evt| {
                            evt.prevent_default();
                            let api_ctx = api_ctx_for_webhook_submit.clone();
                            let mut toast = toast;
                            let url = webhook_url.read().trim().to_string();
                            let events = webhook_events.read().clone();
                            let active = *webhook_active.read();
                            let editing_id = webhook_edit_id.read().clone();
                            if url.is_empty() || events.is_empty() {
                                toast.push(ToastLevel::Warning, "Webhook incomplete", "Enter a URL and at least one event.");
                                return;
                            }
                            webhook_saving.set(true);
                            spawn(async move {
                                let result = if let Some(id) = editing_id {
                                    api::update_admin_webhook(
                                        &api_ctx,
                                        &id,
                                        &UpdateAdminWebhookSubscriptionBody {
                                            url: Some(url.clone()),
                                            events: Some(events.clone()),
                                            active: Some(active),
                                        },
                                    )
                                    .await
                                    .map(|_| None)
                                } else {
                                    api::create_admin_webhook(&api_ctx, &url, &events)
                                        .await
                                        .map(|created| Some(created.secret))
                                };
                                match result {
                                    Ok(secret) => {
                                        webhook_url.set(String::new());
                                        webhook_events.set(vec!["course.published".to_string()]);
                                        webhook_active.set(true);
                                        webhook_edit_id.set(None);
                                        webhook_secret.set(secret);
                                        toast.push(ToastLevel::Success, "Webhook saved", "The webhook subscription was updated.");
                                        webhooks.restart();
                                    }
                                    Err(err) => toast.push(ToastLevel::Danger, "Save failed", format!("{err}")),
                                }
                                webhook_saving.set(false);
                            });
                        },
                        h2 { if editing { "Edit webhook" } else { "Create webhook" } }
                        label { class: "integration-label", "Endpoint URL" }
                        Input {
                            value: webhook_url.read().clone(),
                            input_type: "url".to_string(),
                            placeholder: "https://hooks.example.com/aula".to_string(),
                            disabled: *webhook_saving.read(),
                            on_input: move |value| webhook_url.set(value),
                        }
                        fieldset { class: "integration-check-grid integration-check-grid--events",
                            legend { "Events" }
                            for event in api::ADMIN_WEBHOOK_EVENTS {
                                {event_checkbox(event, webhook_events, *webhook_saving.read())}
                            }
                        }
                        Switch {
                            checked: *webhook_active.read(),
                            disabled: *webhook_saving.read(),
                            label: "Active".to_string(),
                            on_change: move |value| webhook_active.set(value),
                        }
                        if let Some(value) = webhook_secret.read().as_ref() {
                            {secret_panel("Signing secret", value)}
                        }
                        div { class: "integration-form-actions",
                            Button {
                                label: if editing { "Save webhook".to_string() } else { "Create webhook".to_string() },
                                button_type: "submit".to_string(),
                                loading: *webhook_saving.read(),
                                on_click: |_| {},
                            }
                            if editing {
                                Button {
                                    label: "Cancel".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    disabled: *webhook_saving.read(),
                                    on_click: move |_| {
                                        webhook_edit_id.set(None);
                                        webhook_url.set(String::new());
                                        webhook_events.set(vec!["course.published".to_string()]);
                                        webhook_active.set(true);
                                    },
                                }
                            }
                        }
                    }
                    div { class: "integration-table-panel",
                        h2 { "Webhook endpoints" }
                        {hooks_table}
                    }
                }
                div { class: "integration-table-panel integration-full-panel",
                    h2 { "Recent deliveries" }
                    {deliveries_table}
                }
                {delivery_detail(delivery_sheet_open, selected_delivery.read().clone())}
            }
        }
    };

    let sso_section: Element = {
        let snap = sso_config.read_unchecked();
        let current: Option<AdminSsoConfigDto> = snap
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .cloned()
            .flatten();
        let load_state = match snap.as_ref() {
            Some(Err(e)) => rsx! { p { class: "error", "Could not load SSO config: {e}" } },
            None => rsx! { SkeletonCard { height: "120px".to_string() } },
            _ => rsx! {},
        };
        let current_secret = current
            .as_ref()
            .map(|cfg| cfg.has_client_secret)
            .unwrap_or(false);
        let login_url = current
            .as_ref()
            .map(|cfg| cfg.login_url.clone())
            .unwrap_or_else(|| "Save SSO settings to generate the tenant login URL.".to_string());
        let api_ctx_for_sso_submit = api_ctx.clone();
        rsx! {
            section { class: "integration-section",
                {load_state}
                div { class: "integration-grid",
                    form {
                        class: "integration-form integration-form--wide",
                        onsubmit: move |evt| {
                            evt.prevent_default();
                            let api_ctx = api_ctx_for_sso_submit.clone();
                            let mut toast = toast;
                            let secret = sso_secret.read().trim().to_string();
                            let body = UpsertAdminSsoConfigBody {
                                issuer: sso_issuer.read().trim().to_string(),
                                client_id: sso_client_id.read().trim().to_string(),
                                client_secret: if secret.is_empty() { None } else { Some(secret) },
                                authorize_url: sso_authorize_url.read().trim().to_string(),
                                token_url: sso_token_url.read().trim().to_string(),
                                jwks_url: sso_jwks_url.read().trim().to_string(),
                                enabled: *sso_enabled.read(),
                            };
                            sso_saving.set(true);
                            spawn(async move {
                                match api::upsert_admin_sso_config(&api_ctx, &body).await {
                                    Ok(_) => {
                                        sso_secret.set(String::new());
                                        toast.push(ToastLevel::Success, "SSO saved", "Tenant SSO settings were updated.");
                                        sso_config.restart();
                                    }
                                    Err(err) => toast.push(ToastLevel::Danger, "Save failed", format!("{err}")),
                                }
                                sso_saving.set(false);
                            });
                        },
                        h2 { "SSO configuration" }
                        div { class: "integration-form-columns",
                            div {
                                label { class: "integration-label", "Issuer" }
                                Input {
                                    value: sso_issuer.read().clone(),
                                    input_type: "url".to_string(),
                                    placeholder: "https://idp.example.com".to_string(),
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_issuer.set(value),
                                }
                            }
                            div {
                                label { class: "integration-label", "Client ID" }
                                Input {
                                    value: sso_client_id.read().clone(),
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_client_id.set(value),
                                }
                            }
                            div {
                                label { class: "integration-label", "Client secret" }
                                Input {
                                    value: sso_secret.read().clone(),
                                    input_type: "password".to_string(),
                                    placeholder: if current_secret { "Stored secret will be preserved".to_string() } else { "Required".to_string() },
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_secret.set(value),
                                }
                            }
                            div {
                                label { class: "integration-label", "Authorize URL" }
                                Input {
                                    value: sso_authorize_url.read().clone(),
                                    input_type: "url".to_string(),
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_authorize_url.set(value),
                                }
                            }
                            div {
                                label { class: "integration-label", "Token URL" }
                                Input {
                                    value: sso_token_url.read().clone(),
                                    input_type: "url".to_string(),
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_token_url.set(value),
                                }
                            }
                            div {
                                label { class: "integration-label", "JWKS URL" }
                                Input {
                                    value: sso_jwks_url.read().clone(),
                                    input_type: "url".to_string(),
                                    disabled: *sso_saving.read(),
                                    on_input: move |value| sso_jwks_url.set(value),
                                }
                            }
                        }
                        Switch {
                            checked: *sso_enabled.read(),
                            disabled: *sso_saving.read(),
                            label: "Enabled".to_string(),
                            on_change: move |value| sso_enabled.set(value),
                        }
                        div { class: "integration-endpoint-box",
                            span { "Login URL" }
                            code { "{login_url}" }
                        }
                        Button {
                            label: "Save SSO".to_string(),
                            button_type: "submit".to_string(),
                            loading: *sso_saving.read(),
                            on_click: |_| {},
                        }
                    }
                }
            }
        }
    };

    let lti_section: Element = {
        let snap = lti_platforms.read_unchecked();
        let table = match snap.as_ref() {
            Some(Ok(rows)) => {
                let filtered: Vec<AdminLtiPlatformDto> = rows
                    .iter()
                    .filter(|platform| {
                        matches_any(
                            &[
                                &platform.name,
                                &platform.issuer,
                                &platform.client_id,
                                &platform.deployment_id,
                            ],
                            &current_query,
                        )
                    })
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    rsx! { p { class: "muted", "No LTI platforms match the current filter." } }
                } else {
                    let api_ctx = api_ctx.clone();
                    let mut lti_platforms = lti_platforms;
                    lti_table(
                        &filtered,
                        EventHandler::new(move |id: String| {
                            if !confirm_action("Delete this LTI platform?") {
                                return;
                            }
                            let api_ctx = api_ctx.clone();
                            let mut toast = toast;
                            spawn(async move {
                                match api::delete_admin_lti_platform(&api_ctx, &id).await {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "LTI platform deleted",
                                            "The platform registration was removed.",
                                        );
                                        lti_platforms.restart();
                                    }
                                    Err(err) => toast.push(
                                        ToastLevel::Danger,
                                        "Delete failed",
                                        format!("{err}"),
                                    ),
                                }
                            });
                        }),
                    )
                }
            }
            Some(Err(e)) => rsx! { p { class: "error", "Could not load LTI platforms: {e}" } },
            None => rsx! { SkeletonCard { height: "180px".to_string() } },
        };
        let api_ctx_for_lti_submit = api_ctx.clone();
        rsx! {
            section { class: "integration-section",
                div { class: "integration-grid",
                    form {
                        class: "integration-form",
                        onsubmit: move |evt| {
                            evt.prevent_default();
                            let api_ctx = api_ctx_for_lti_submit.clone();
                            let mut toast = toast;
                            let default_course = lti_default_course_id.read().trim().to_string();
                            let body = RegisterAdminLtiPlatformBody {
                                name: lti_name.read().trim().to_string(),
                                issuer: lti_issuer.read().trim().to_string(),
                                client_id: lti_client_id.read().trim().to_string(),
                                auth_login_url: lti_auth_login_url.read().trim().to_string(),
                                jwks_url: lti_jwks_url.read().trim().to_string(),
                                deployment_id: lti_deployment_id.read().trim().to_string(),
                                default_course_id: if default_course.is_empty() { None } else { Some(default_course) },
                            };
                            lti_saving.set(true);
                            spawn(async move {
                                match api::register_admin_lti_platform(&api_ctx, &body).await {
                                    Ok(_) => {
                                        lti_name.set(String::new());
                                        lti_issuer.set(String::new());
                                        lti_client_id.set(String::new());
                                        lti_auth_login_url.set(String::new());
                                        lti_jwks_url.set(String::new());
                                        lti_deployment_id.set(String::new());
                                        lti_default_course_id.set(String::new());
                                        toast.push(ToastLevel::Success, "LTI platform registered", "The platform can now launch into AulaLite.");
                                        lti_platforms.restart();
                                    }
                                    Err(err) => toast.push(ToastLevel::Danger, "Register failed", format!("{err}")),
                                }
                                lti_saving.set(false);
                            });
                        },
                        h2 { "Register LTI platform" }
                        label { class: "integration-label", "Name" }
                        Input { value: lti_name.read().clone(), disabled: *lti_saving.read(), on_input: move |v| lti_name.set(v) }
                        label { class: "integration-label", "Issuer" }
                        Input { value: lti_issuer.read().clone(), input_type: "url".to_string(), disabled: *lti_saving.read(), on_input: move |v| lti_issuer.set(v) }
                        label { class: "integration-label", "Client ID" }
                        Input { value: lti_client_id.read().clone(), disabled: *lti_saving.read(), on_input: move |v| lti_client_id.set(v) }
                        label { class: "integration-label", "Auth login URL" }
                        Input { value: lti_auth_login_url.read().clone(), input_type: "url".to_string(), disabled: *lti_saving.read(), on_input: move |v| lti_auth_login_url.set(v) }
                        label { class: "integration-label", "JWKS URL" }
                        Input { value: lti_jwks_url.read().clone(), input_type: "url".to_string(), disabled: *lti_saving.read(), on_input: move |v| lti_jwks_url.set(v) }
                        label { class: "integration-label", "Deployment ID" }
                        Input { value: lti_deployment_id.read().clone(), disabled: *lti_saving.read(), on_input: move |v| lti_deployment_id.set(v) }
                        label { class: "integration-label", "Default course ID" }
                        Input { value: lti_default_course_id.read().clone(), disabled: *lti_saving.read(), on_input: move |v| lti_default_course_id.set(v) }
                        div { class: "integration-endpoint-box",
                            span { "OIDC login endpoint" }
                            code { "/v1/lti/login" }
                        }
                        Button {
                            label: "Register platform".to_string(),
                            button_type: "submit".to_string(),
                            loading: *lti_saving.read(),
                            on_click: |_| {},
                        }
                    }
                    div { class: "integration-table-panel",
                        h2 { "LTI platforms" }
                        {table}
                    }
                }
            }
        }
    };

    let current_section = match active.as_str() {
        "webhooks" => webhook_section,
        "sso" => sso_section,
        "lti" => lti_section,
        _ => api_key_section,
    };

    let body = rsx! {
        div { class: "admin-integrations-page",
            PageHeader {
                title: "Integrations".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: "Configure tenant API access, event delivery, single sign-on, and LTI launches.".to_string(),
            }
            {summary_grid(counts)}
            div { class: "integration-toolbar",
                Tabs {
                    tabs,
                    active: active_tab.read().clone(),
                    variant: TabsVariant::Pill,
                    on_change: move |key| active_tab.set(key),
                }
                Input {
                    value: search.read().clone(),
                    placeholder: "Filter current section".to_string(),
                    on_input: move |value| search.set(value),
                }
            }
            {current_section}
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
            {body}
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn summary_grid_renders_counts() {
        fn app() -> Element {
            rsx! {
                {summary_grid(IntegrationCounts {
                    active_keys: 2,
                    webhook_count: 3,
                    sso_enabled: true,
                    lti_count: 1,
                })}
            }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Active API keys"));
        assert!(html.contains("Enabled"));
        assert!(html.contains("LTI platforms"));
    }

    #[test]
    fn api_key_table_renders_status_and_scopes() {
        let key = AdminApiKeyDto {
            id: "key-1".into(),
            name: "Reporting".into(),
            prefix: "ak_123".into(),
            scopes: vec!["courses:read".into()],
            created_by: "user-1".into(),
            created_at: "2026-06-18T00:00:00Z".into(),
            last_used_at: None,
            revoked_at: None,
        };
        fn app(key: AdminApiKeyDto) -> Element {
            rsx! { {api_key_table(&[key], EventHandler::new(|_: String| {}))} }
        }
        let mut dom = VirtualDom::new_with_props(app, key);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Reporting"));
        assert!(html.contains("courses:read"));
        assert!(html.contains("Active"));
    }

    #[test]
    fn webhook_delivery_table_renders_payload_open_action() {
        let row = AdminWebhookDeliveryDto {
            id: "delivery-1".into(),
            subscription_id: "sub-1".into(),
            event: "submission.graded".into(),
            payload_json: serde_json::json!({ "score": 9 }),
            status: "failed".into(),
            attempts: 2,
            last_attempt_at: None,
            response_code: Some(500),
            created_at: "2026-06-18T00:00:00Z".into(),
        };
        fn app(row: AdminWebhookDeliveryDto) -> Element {
            rsx! { {webhook_delivery_table(&[row], EventHandler::new(|_: AdminWebhookDeliveryDto| {}))} }
        }
        let mut dom = VirtualDom::new_with_props(app, row);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("submission.graded"));
        assert!(html.contains("failed"));
        assert!(html.contains("Open"));
    }

    #[test]
    fn lti_table_renders_platform_fields() {
        let platform = AdminLtiPlatformDto {
            id: "platform-1".into(),
            name: "Canvas".into(),
            issuer: "https://canvas.example.com".into(),
            client_id: "client-1".into(),
            auth_login_url: "https://canvas.example.com/login".into(),
            jwks_url: "https://canvas.example.com/jwks".into(),
            deployment_id: "deployment-1".into(),
            default_course_id: None,
            created_at: "2026-06-18T00:00:00Z".into(),
        };
        fn app(platform: AdminLtiPlatformDto) -> Element {
            rsx! { {lti_table(&[platform], EventHandler::new(|_: String| {}))} }
        }
        let mut dom = VirtualDom::new_with_props(app, platform);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("Canvas"));
        assert!(html.contains("client-1"));
        assert!(html.contains("deployment-1"));
    }

    #[test]
    fn payload_preview_formats_json() {
        let out = payload_preview(&serde_json::json!({ "score": 9 }));
        assert!(out.contains("score"));
        assert!(out.contains("9"));
    }
}
