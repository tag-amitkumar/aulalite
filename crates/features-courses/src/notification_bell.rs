// crates/features-courses/src/notification_bell.rs
//! Topbar notification bell with an unread-count badge and a dropdown of
//! recent notifications. Self-contained: reads `ApiContext` from context, so
//! it can be dropped into the AppShell topbar without prop plumbing.
//!
//! Polling: mirrors the `active_session` poll pattern — every ~30s on success,
//! backing off to 60s after a transient error, and stopping entirely on 401
//! (the ApiContext refresh path resumes on the next user interaction). The
//! loop is wasm-only in practice; off-wasm it runs a single iteration so SSR
//! tests that mount the bell don't spin.
//!
//! The dropdown follows the project-wide four-state UX: loading, empty
//! ("No notifications"), error, and loaded. Each item links to
//! `notification.link` (when present) and marks itself read on click; a
//! "Mark all read" action clears the badge.

use crate::api::{self, ApiContext, ApiError, NotificationDto};
use design_system::{Badge, BadgeSize, BadgeTone, Button, ButtonSize, ButtonVariant, EmptyState};
use dioxus::prelude::*;

/// Loaded list of recent notifications used by the dropdown body.
#[derive(Clone, Debug, PartialEq)]
enum ListState {
    Loading,
    Loaded(Vec<NotificationDto>),
    Error(String),
}

/// Best-effort human-readable "time ago" from an RFC3339 timestamp string.
/// Pure so it's unit-testable. Falls back to the raw string when the timestamp
/// can't be parsed.
fn relative_time(created_at: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let parsed = match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return created_at.to_string(),
    };
    let secs = (now - parsed).num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d ago");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return format!("{weeks}w ago");
    }
    // Older than a month: show the calendar date.
    parsed.format("%b %-d, %Y").to_string()
}

/// How many recent notifications to fetch when the dropdown opens.
const DROPDOWN_LIMIT: i64 = 20;

#[component]
#[allow(clippy::never_loop)]
pub fn NotificationBell() -> Element {
    // Resolve ApiContext defensively: the bell is mounted in the AppShell
    // topbar which renders on every authed route (the signal is always
    // provided there), but tolerating its absence keeps the bell panic-free in
    // SSR tests / unusual mount sites. A missing context yields an empty token,
    // and the poll short-circuits on the resulting 401.
    let api = use_hook(|| {
        try_consume_context::<Signal<ApiContext>>().unwrap_or_else(|| {
            Signal::new(ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            })
        })
    });
    let mut unread = use_signal(|| 0_i64);
    let mut open = use_signal(|| false);
    let mut list = use_signal(|| ListState::Loading);

    // --- Unread-count poll (copies active_session cadence/teardown). ---
    use_future(move || {
        let api = api;
        async move {
            loop {
                #[cfg(target_arch = "wasm32")]
                if crate::browser_runtime::page_is_hidden() {
                    gloo_timers::future::TimeoutFuture::new(30_000).await;
                    continue;
                }

                let ctx = api.read().clone();
                let delay_ms = match api::notifications_unread_count(&ctx).await {
                    Ok(count) => {
                        unread.set(count);
                        30_000_u32
                    }
                    Err(ApiError::Status(401, _)) => {
                        // Stop polling; the refresh path resumes later.
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "unread-count poll failed; backing off 60s");
                        60_000_u32
                    }
                };
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(delay_ms).await;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Host build: poll loop is wasm-only in practice. Return
                    // after one iteration so SSR tests don't spin.
                    let _ = delay_ms;
                    return;
                }
            }
        }
    });

    // Load (or reload) the dropdown list from the server.
    let mut load_list = move || {
        list.set(ListState::Loading);
        spawn(async move {
            let ctx = api.read().clone();
            match api::list_notifications(&ctx, Some(DROPDOWN_LIMIT)).await {
                Ok(items) => list.set(ListState::Loaded(items)),
                Err(e) => list.set(ListState::Error(format!("{e}"))),
            }
        });
    };

    let toggle_open = move |_| {
        let next = !*open.read();
        open.set(next);
        if next {
            load_list();
        }
    };

    let mark_all = move |_| {
        spawn(async move {
            let ctx = api.read().clone();
            if api::mark_all_notifications_read(&ctx).await.is_ok() {
                unread.set(0);
                // Reflect read state locally without a round-trip. Snapshot the
                // current items out of the read borrow before mutating the
                // signal to avoid a borrow conflict.
                let snapshot = match &*list.read() {
                    ListState::Loaded(items) => Some(items.clone()),
                    _ => None,
                };
                if let Some(items) = snapshot {
                    let cleared: Vec<NotificationDto> = items
                        .into_iter()
                        .map(|mut n| {
                            if n.read_at.is_none() {
                                n.read_at = Some(String::new());
                            }
                            n
                        })
                        .collect();
                    list.set(ListState::Loaded(cleared));
                }
            }
        });
    };

    let unread_count = *unread.read();
    let is_open = *open.read();
    let now = chrono::Utc::now();

    let dropdown_body: Element = match &*list.read() {
        ListState::Loading => rsx! {
            div { class: "notif-dropdown-loading", "Loading…" }
        },
        ListState::Error(e) => rsx! {
            p { class: "error notif-dropdown-error", "Could not load notifications: {e}" }
        },
        ListState::Loaded(items) if items.is_empty() => rsx! {
            EmptyState {
                title: "No notifications".to_string(),
                description: "You're all caught up.".to_string(),
            }
        },
        ListState::Loaded(items) => {
            let rows = items.clone();
            rsx! {
                ul { class: "notif-list", role: "list",
                    for n in rows.into_iter() {
                        {
                            let id = n.id.clone();
                            let unread_item = n.read_at.is_none();
                            let item_class = if unread_item {
                                "notif-item notif-item--unread"
                            } else {
                                "notif-item"
                            };
                            let when = relative_time(&n.created_at, now);
                            let link = n.link.clone();
                            let body = n.body.clone();
                            let title = n.title.clone();
                            // Mark this item read (best-effort); the <a href>
                            // default action handles navigation when a link
                            // is present.
                            let on_item_click = move |_| {
                                let id = id.clone();
                                spawn(async move {
                                    let ctx = api.read().clone();
                                    if api::mark_notification_read(&ctx, &id).await.is_ok() {
                                        let cur = *unread.read();
                                        if cur > 0 {
                                            unread.set(cur - 1);
                                        }
                                    }
                                });
                            };
                            let href = link.clone().unwrap_or_else(|| "#".to_string());
                            rsx! {
                                li { class: "{item_class}",
                                    a {
                                        class: "notif-item-link",
                                        href: "{href}",
                                        onclick: on_item_click,
                                        div { class: "notif-item-head",
                                            span { class: "notif-item-title", "{title}" }
                                            if unread_item {
                                                span { class: "notif-item-dot", "aria-hidden": "true" }
                                            }
                                        }
                                        if let Some(b) = body {
                                            p { class: "notif-item-body", "{b}" }
                                        }
                                        span { class: "notif-item-time", "{when}" }
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
        div { class: "notif-bell",
            button {
                class: "notif-bell-trigger",
                r#type: "button",
                "aria-label": "Notifications",
                "aria-haspopup": "true",
                "aria-expanded": if is_open { "true" } else { "false" },
                onclick: toggle_open,
                // Bell glyph (inline SVG; avoids adding a new UiIcon variant in
                // design-system, which is out of edit scope for this task).
                svg {
                    class: "notif-bell-icon",
                    width: "20",
                    height: "20",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    "stroke-width": "2",
                    "stroke-linecap": "round",
                    "stroke-linejoin": "round",
                    "aria-hidden": "true",
                    path { d: "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" }
                    path { d: "M10.3 21a1.94 1.94 0 0 0 3.4 0" }
                }
                if unread_count > 0 {
                    span { class: "notif-bell-badge",
                        Badge {
                            label: if unread_count > 99 { "99+".to_string() } else { unread_count.to_string() },
                            tone: BadgeTone::Danger,
                            size: BadgeSize::Sm,
                        }
                    }
                }
            }
            if is_open {
                div {
                    class: "notif-dropdown",
                    role: "menu",
                    "aria-label": "Notifications",
                    div { class: "notif-dropdown-head",
                        span { class: "notif-dropdown-title", "Notifications" }
                        Button {
                            label: "Mark all read".to_string(),
                            variant: ButtonVariant::Link,
                            size: ButtonSize::Sm,
                            disabled: unread_count == 0,
                            on_click: mark_all,
                        }
                    }
                    div { class: "notif-dropdown-body", {dropdown_body} }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ApiContext {
        ApiContext {
            base_url: String::new(),
            id_token: String::new(),
        }
    }

    /// Mount the bell with an ApiContext in scope so it renders (off-wasm the
    /// poll loop runs a single no-op iteration).
    fn app() -> Element {
        use_context_provider(|| Signal::new(ctx()));
        rsx! { NotificationBell {} }
    }

    #[test]
    fn bell_renders_trigger_with_aria_label() {
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("notif-bell"),
            "bell container missing: {html}"
        );
        assert!(
            html.contains("aria-label=\"Notifications\""),
            "aria-label missing: {html}"
        );
        // The bell glyph is an inline SVG.
        assert!(html.contains("<svg"), "bell svg missing: {html}");
        // Dropdown is closed by default — no menu rendered.
        assert!(
            !html.contains("notif-dropdown"),
            "dropdown should be closed by default: {html}"
        );
    }

    #[test]
    fn relative_time_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-29T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(relative_time("2026-05-29T11:59:30Z", now), "just now");
        assert_eq!(relative_time("2026-05-29T11:30:00Z", now), "30m ago");
        assert_eq!(relative_time("2026-05-29T09:00:00Z", now), "3h ago");
        assert_eq!(relative_time("2026-05-27T12:00:00Z", now), "2d ago");
        assert_eq!(relative_time("2026-05-15T12:00:00Z", now), "2w ago");
        // Older than a month → calendar date.
        assert_eq!(relative_time("2026-03-01T12:00:00Z", now), "Mar 1, 2026");
        // Unparseable → echoed back.
        assert_eq!(relative_time("not-a-date", now), "not-a-date");
        // Future timestamp → "just now" (clock skew tolerance).
        assert_eq!(relative_time("2026-05-29T13:00:00Z", now), "just now");
    }
}
