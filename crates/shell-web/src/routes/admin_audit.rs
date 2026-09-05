// crates/shell-web/src/routes/admin_audit.rs
//
// Admin audit-log viewer. Restricted to org-admin / platform-admin users.
// Renders most-recent events first as a kinetics `DataTable` and supports
// cursor pagination: `list_audit_events` accepts a `before` cursor and returns
// events oldest-bound by it. The cursor for the next page is the `occurred_at`
// of the last (oldest) event currently loaded. "Load more" appends the next
// page; when a page returns fewer than the requested limit we stop offering it.
use design_system::kinetics_ui::{DataTable, DataTableColumn, DataTableRow};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::AuditEventDto;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

const PAGE_SIZE: i64 = 50;

/// Format an RFC3339 timestamp as `MMM D, YYYY h:mm AM/PM`, falling back to
/// the raw string when parsing fails (same pattern as parent_home::format_ts).
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Humanize one snake_case enum segment: underscores become spaces and the
/// first letter is capitalized ("go_live" → "Go live").
fn humanize_segment(segment: &str) -> String {
    let spaced = segment.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Humanize a dotted action enum into a readable label:
/// "live_session.go_live" → "Live session · Go live".
fn humanize_action(action: &str) -> String {
    action
        .split('.')
        .filter(|s| !s.is_empty())
        .map(humanize_segment)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Shorten an opaque resource id for display: first 8 chars + "…" when longer.
fn short_id(id: &str) -> String {
    let prefix: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        format!("{prefix}…")
    } else {
        prefix
    }
}

/// Build the audit DataTable. Pure so it's SSR-testable.
fn audit_table(events: &[AuditEventDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("when", "When"),
        DataTableColumn::new("actor", "Actor"),
        DataTableColumn::new("action", "Action"),
        DataTableColumn::new("resource", "Resource"),
    ];
    let rows: Vec<DataTableRow> = events
        .iter()
        .map(|e| {
            let actor = e
                .actor_display_name
                .as_deref()
                .or(e.actor_email.as_deref())
                .unwrap_or("(unknown)")
                .to_string();
            DataTableRow::new(
                e.id.clone(),
                vec![
                    format_ts(&e.occurred_at),
                    actor,
                    humanize_action(&e.action),
                    format!(
                        "{} · {}",
                        humanize_segment(&e.resource_type),
                        short_id(&e.resource_id)
                    ),
                ],
            )
        })
        .collect();
    rsx! {
        DataTable { columns, rows }
    }
}

#[component]
pub fn AdminAudit() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

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
                p { "You don't have permission to view the audit log." }
            }
        };
    }

    // Accumulated events across pages, plus pagination state.
    let mut events = use_signal(Vec::<AuditEventDto>::new);
    let mut loading = use_signal(|| true);
    let mut load_error = use_signal(|| None::<String>);
    // Whether more pages may exist (last page was full).
    let mut has_more = use_signal(|| false);
    let mut loaded_once = use_signal(|| false);

    // Initial load (runs once).
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
                match api::list_audit_events(&api, Some(PAGE_SIZE), None).await {
                    Ok(resp) => {
                        has_more.set(resp.events.len() as i64 >= PAGE_SIZE);
                        events.set(resp.events);
                    }
                    Err(e) => load_error.set(Some(format!("{e}"))),
                }
                loading.set(false);
            });
        });
    }

    // "Load more" handler: fetch the next page using the oldest event's
    // occurred_at as the `before` cursor, then append.
    let api_more = api.clone();
    let load_more = move |_| {
        let api = api_more.clone();
        let cursor = events.read().last().map(|e| e.occurred_at.clone());
        let Some(cursor) = cursor else { return };
        spawn(async move {
            loading.set(true);
            load_error.set(None);
            match api::list_audit_events(&api, Some(PAGE_SIZE), Some(&cursor)).await {
                Ok(resp) => {
                    has_more.set(resp.events.len() as i64 >= PAGE_SIZE);
                    events.write().extend(resp.events);
                }
                Err(e) => load_error.set(Some(format!("{e}"))),
            }
            loading.set(false);
        });
    };

    let is_loading = *loading.read();
    let err = load_error.read().clone();
    let evs = events.read().clone();
    let more = *has_more.read();

    let body: Element = match (&err, is_loading, evs.is_empty()) {
        // Error with no data loaded.
        (Some(e), _, true) => rsx! {
            div { class: "admin-audit-page",
                h1 { "Audit log" }
                p { class: "error", "Could not load audit log: {e}" }
            }
        },
        // Initial loading (no data yet).
        (None, true, true) => rsx! {
            div { class: "admin-audit-page",
                h1 { "Audit log" }
                p { "Loading audit log…" }
            }
        },
        // Loaded but empty.
        (None, false, true) => rsx! {
            div { class: "admin-audit-page",
                h1 { "Audit log" }
                p { class: "muted",
                    "No events recorded yet. Significant actions (enrollment, file upload, recording publish, role change) will appear here as they occur."
                }
            }
        },
        // Loaded with data (possibly mid-load-more or with a soft error).
        _ => {
            let table = audit_table(&evs);
            rsx! {
                div { class: "admin-audit-page",
                    h1 { "Audit log" }
                    p { class: "muted", "{evs.len()} events loaded, most recent first." }
                    {table}
                    if let Some(e) = &err {
                        p { class: "error", "Could not load more: {e}" }
                    }
                    if more {
                        button {
                            class: "linkish",
                            disabled: is_loading,
                            onclick: load_more,
                            if is_loading { "Loading…" } else { "Load more" }
                        }
                    }
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
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    fn sample() -> Vec<AuditEventDto> {
        vec![AuditEventDto {
            id: "ev-1".into(),
            actor_user_id: "u-1".into(),
            actor_email: Some("a@x".into()),
            actor_display_name: Some("Ada".into()),
            action: "course.created".into(),
            resource_type: "course".into(),
            resource_id: "c-1".into(),
            metadata: None,
            occurred_at: "2026-05-29T00:00:00Z".into(),
        }]
    }

    #[test]
    fn audit_table_renders_data_table() {
        fn app_inner(events: Vec<AuditEventDto>) -> Element {
            audit_table(&events)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        assert!(html.contains("Ada"));
        // Action enum and resource are humanized; timestamp is formatted.
        assert!(html.contains("Course · Created"), "got: {html}");
        assert!(html.contains("Course · c-1"), "got: {html}");
        assert!(html.contains("May 29, 2026 12:00 AM"), "got: {html}");
        assert!(html.contains("When"));
        // The duplicate "Audit events" caption under the H1 is gone.
        assert!(!html.contains("ui-data-table-caption"), "got: {html}");
    }

    #[test]
    fn humanize_action_splits_and_capitalizes_segments() {
        assert_eq!(
            humanize_action("live_session.go_live"),
            "Live session · Go live"
        );
        assert_eq!(humanize_action("course.created"), "Course · Created");
        assert_eq!(humanize_action("role_change"), "Role change");
        assert_eq!(humanize_action(""), "");
    }

    #[test]
    fn short_id_truncates_long_ids_only() {
        assert_eq!(short_id("0123456789abcdef"), "01234567…");
        assert_eq!(short_id("c-1"), "c-1");
        assert_eq!(short_id("12345678"), "12345678");
    }

    #[test]
    fn format_ts_falls_back_to_raw_on_parse_failure() {
        assert_eq!(format_ts("not-a-date"), "not-a-date");
        assert_eq!(
            format_ts("2026-06-10T22:48:04.783724Z"),
            "Jun 10, 2026 10:48 PM"
        );
    }
}
