// crates/shell-web/src/routes/admin_notification_deliveries.rs
//
// Admin notification delivery diagnostics. Restricted to org-admin /
// platform-admin users. Shows recent delivery attempts without exposing raw
// device tokens and lets admins inspect provider status/error details.
use design_system::{
    Badge, BadgeTone, Sheet, SheetBody, SheetClose, SheetHeader, SheetTitle, Table,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::DeliveryDto;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

const PAGE_SIZE: i64 = 50;

fn status_tone(status: &str) -> BadgeTone {
    match status {
        "sent" => BadgeTone::Success,
        "failed" => BadgeTone::Danger,
        "skipped" => BadgeTone::Warning,
        "queued" => BadgeTone::Info,
        _ => BadgeTone::Neutral,
    }
}

fn short_hash(hash: &str) -> String {
    let prefix: String = hash.chars().take(10).collect();
    if hash.chars().count() > 10 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn target_label(row: &DeliveryDto) -> String {
    row.target_label
        .clone()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| short_hash(&row.target_hash))
}

fn deliveries_table(deliveries: &[DeliveryDto], on_select: EventHandler<String>) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "When" }
                    th { class: "ds-table-th", "Status" }
                    th { class: "ds-table-th", "Channel" }
                    th { class: "ds-table-th", "Provider" }
                    th { class: "ds-table-th", "Kind" }
                    th { class: "ds-table-th", "Target" }
                    th { class: "ds-table-th", "Details" }
                }
            },
            body: rsx! {
                for row in deliveries.iter() {
                    {
                        let id = row.id.clone();
                        let target = target_label(row);
                        let select = on_select;
                        rsx! {
                            tr { key: "{id}",
                                td { "{row.created_at}" }
                                td { Badge { label: row.status.clone(), tone: status_tone(&row.status) } }
                                td { "{row.channel}" }
                                td { "{row.provider}" }
                                td { "{row.kind}" }
                                td { "{target}" }
                                td {
                                    button {
                                        class: "linkish",
                                        onclick: move |_| select.call(id.clone()),
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

fn delivery_detail(open: Signal<bool>, delivery: Option<DeliveryDto>) -> Element {
    rsx! {
        Sheet { open, width: 520, aria_label: "Notification delivery details",
            SheetHeader {
                SheetTitle { "Delivery details" }
                SheetClose { open }
            }
            SheetBody {
                if let Some(row) = delivery {
                    {
                        let provider_status = row.provider_status.clone().unwrap_or_else(|| "None".into());
                        let error_code = row.error_code.clone().unwrap_or_else(|| "None".into());
                        let error_message = row.error_message.clone().unwrap_or_else(|| "None".into());
                        let target = target_label(&row);
                        rsx! {
                            dl { class: "delivery-detail-list",
                                dt { "Status" } dd { "{row.status}" }
                                dt { "Provider" } dd { "{row.provider}" }
                                dt { "Provider status" } dd { "{provider_status}" }
                                dt { "Error code" } dd { "{error_code}" }
                                dt { "Error message" } dd { "{error_message}" }
                                dt { "Target" } dd { "{target}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn AdminNotificationDeliveries() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting..." } };
        }
    };

    let is_admin = user.can_manage_organization();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to view notification deliveries." }
            }
        };
    }

    let mut deliveries = use_signal(Vec::<DeliveryDto>::new);
    let mut loading = use_signal(|| true);
    let mut load_error = use_signal(|| None::<String>);
    let selected = use_signal(|| None::<DeliveryDto>);
    let sheet_open = use_signal(|| false);
    let mut loaded_once = use_signal(|| false);

    {
        let api = api.clone();
        use_effect(move || {
            if *loaded_once.read() {
                return;
            }
            loaded_once.set(true);
            let api = api.clone();
            spawn(async move {
                loading.set(true);
                load_error.set(None);
                match api::list_notification_deliveries(&api, None, None, Some(PAGE_SIZE)).await {
                    Ok(resp) => deliveries.set(resp.deliveries),
                    Err(err) => load_error.set(Some(format!("{err}"))),
                }
                loading.set(false);
            });
        });
    }

    let open_delivery = {
        let api = api.clone();
        move |id: String| {
            let api = api.clone();
            let mut selected = selected;
            let mut sheet_open = sheet_open;
            spawn(async move {
                if let Ok(row) = api::get_notification_delivery(&api, &id).await {
                    selected.set(Some(row));
                    sheet_open.set(true);
                }
            });
        }
    };

    let err = load_error.read().clone();
    let is_loading = *loading.read();
    let rows = deliveries.read().clone();
    let body: Element = match (&err, is_loading, rows.is_empty()) {
        (Some(e), _, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { class: "error", "Could not load delivery diagnostics: {e}" }
            }
        },
        (None, true, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { "Loading delivery diagnostics..." }
            }
        },
        (None, false, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { class: "muted", "No delivery attempts recorded yet." }
            }
        },
        _ => {
            let open_delivery = open_delivery.clone();
            rsx! {
                div { class: "admin-deliveries-page",
                    h1 { "Notification deliveries" }
                    p { class: "muted", "{rows.len()} delivery attempts loaded." }
                    {deliveries_table(&rows, EventHandler::new(move |id: String| open_delivery(id)))}
                    if let Some(e) = &err {
                        p { class: "error", "Could not refresh delivery diagnostics: {e}" }
                    }
                    {delivery_detail(sheet_open, selected.read().clone())}
                }
            }
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

    fn sample() -> Vec<DeliveryDto> {
        vec![DeliveryDto {
            id: "delivery-1".into(),
            user_id: "user-1".into(),
            notification_id: None,
            channel: "push".into(),
            provider: "fcm".into(),
            target_hash: "abcdef123456".into(),
            target_label: Some("web - Chrome".into()),
            device_token_id: None,
            kind: "grade_released".into(),
            status: "failed".into(),
            provider_message_id: None,
            provider_status: Some("404".into()),
            error_code: Some("UNREGISTERED".into()),
            error_message: Some("token is not registered".into()),
            created_at: "2026-06-18T00:00:00Z".into(),
            updated_at: "2026-06-18T00:00:00Z".into(),
        }]
    }

    #[test]
    fn deliveries_table_renders_status_and_sanitized_target() {
        fn app() -> Element {
            deliveries_table(&sample(), EventHandler::new(|_| {}))
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("failed"), "{html}");
        assert!(html.contains("web - Chrome"), "{html}");
        assert!(html.contains("grade_released"), "{html}");
        assert!(!html.contains("tok-"), "{html}");
    }

    #[test]
    fn delivery_detail_renders_provider_errors() {
        fn app() -> Element {
            let open = use_signal(|| true);
            delivery_detail(open, sample().into_iter().next())
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Delivery details"), "{html}");
        assert!(html.contains("UNREGISTERED"), "{html}");
        assert!(html.contains("token is not registered"), "{html}");
    }

    #[test]
    fn short_hash_truncates_long_hashes() {
        assert_eq!(short_hash("abcdef123456"), "abcdef1234...");
        assert_eq!(short_hash("abc"), "abc");
    }
}
