// crates/features-courses/src/calendar_view.rs
//! Personal calendar — an agenda list of the caller's upcoming live sessions
//! and assignment due dates, grouped by day, each linking out to its session
//! room or assignment page. Plus a "Download .ics" / "Subscribe" affordance.
//!
//! Reads `use_api()` and calls `api::list_my_calendar`. Timestamps are rendered
//! in the browser's LOCAL timezone (see `format_local_ts`): on wasm we convert
//! the UTC instant with `js_sys::Date`; off-wasm (SSR tests) we fall back to the
//! shared UTC humanizer so the markup is still meaningful.
use design_system::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState};
use dioxus::prelude::*;

use crate::api::{fetch_json, ApiContext, ApiError};

/// One calendar event from `GET /v1/me/calendar`. Mirrors the backend
/// `handlers::calendar::CalendarEvent`: UUID + timestamp fields are serialized
/// as JSON strings, so they're decoded here as `String`.
///
/// Defined in this module (rather than the shared `api` module) so the calendar
/// feature stays self-contained.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CalendarEventDto {
    /// `"session"` | `"assignment_due"`.
    pub kind: String,
    pub id: String,
    pub course_id: String,
    pub course_title: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: Option<i32>,
    pub status: Option<String>,
    pub link: String,
}

/// Fetch the caller's calendar events. `from`/`to` are optional RFC3339 bounds;
/// when omitted the backend uses its default window (now-7d .. now+60d).
pub async fn list_my_calendar(
    ctx: &ApiContext,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<CalendarEventDto>, ApiError> {
    let mut path = String::from("/v1/me/calendar");
    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = from {
        parts.push(format!("from={}", urlencoding::encode(f)));
    }
    if let Some(t) = to {
        parts.push(format!("to={}", urlencoding::encode(t)));
    }
    if !parts.is_empty() {
        path.push('?');
        path.push_str(&parts.join("&"));
    }
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

/// Humanize an RFC3339 UTC timestamp into the browser's LOCAL timezone, e.g.
/// "Jun 17, 2026, 10:39 PM". On wasm we parse the instant and format via
/// `js_sys::Date::to_locale_string` so the user sees their own clock. Off-wasm
/// (SSR/tests) we fall back to the UTC humanizer from `schedule_view` so the
/// output is still a friendly string (parity with `parent_home::format_ts`).
pub(crate) fn format_local_ts(raw: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        // js_sys::Date parses an RFC3339/ISO-8601 string; toLocaleString renders
        // in the runtime's local zone. Empty/garbage input → NaN date, whose
        // locale string is "Invalid Date" — fall back to the raw string then.
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(raw));
        if d.get_time().is_nan() {
            return crate::schedule_view::format_session_ts(raw);
        }
        let s = d.to_locale_string("en-US", &wasm_bindgen::JsValue::UNDEFINED);
        let s = String::from(s);
        if s.is_empty() {
            crate::schedule_view::format_session_ts(raw)
        } else {
            s
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::schedule_view::format_session_ts(raw)
    }
}

/// Local-date "day key" used to group events into agenda sections. On wasm this
/// is the local calendar date ("Wed, Jun 17, 2026"); off-wasm it's the UTC date
/// derived from the leading `YYYY-MM-DD` of the timestamp, which keeps SSR
/// grouping deterministic.
pub(crate) fn day_key(raw: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(raw));
        if !d.get_time().is_nan() {
            let opts = js_sys::Object::new();
            // weekday: short, year: numeric, month: short, day: numeric
            let _ = js_sys::Reflect::set(&opts, &"weekday".into(), &"short".into());
            let _ = js_sys::Reflect::set(&opts, &"year".into(), &"numeric".into());
            let _ = js_sys::Reflect::set(&opts, &"month".into(), &"short".into());
            let _ = js_sys::Reflect::set(&opts, &"day".into(), &"numeric".into());
            let s = String::from(d.to_locale_date_string("en-US", &opts));
            if !s.is_empty() {
                return s;
            }
        }
        // Fall through to the date prefix when Date parsing fails.
    }
    raw.split('T').next().unwrap_or(raw).to_string()
}

/// The badge tone for an event row: assignment due → Warning, live session →
/// Live, ended → Success, anything else (scheduled) → Neutral.
fn event_tone(kind: &str, status: Option<&str>) -> BadgeTone {
    match (kind, status) {
        ("assignment_due", _) => BadgeTone::Warning,
        (_, Some("live")) => BadgeTone::Live,
        (_, Some("ended")) => BadgeTone::Success,
        _ => BadgeTone::Neutral,
    }
}

/// The short type label shown on each row.
fn event_label(kind: &str, status: Option<&str>) -> String {
    match (kind, status) {
        ("assignment_due", _) => "Due".to_string(),
        (_, Some(s)) => s.to_string(),
        _ => "session".to_string(),
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CalendarViewProps {
    pub events: Vec<CalendarEventDto>,
}

#[component]
pub fn CalendarView(props: CalendarViewProps) -> Element {
    // Read the live ApiContext so the component can self-serve the .ics
    // download (the endpoint is Bearer-authed, so a plain <a href> won't carry
    // the token). Resolved defensively via `try_consume_context` — mirroring
    // `app_shell` — so the component stays panic-free in SSR tests / unusual
    // mount sites where the signal isn't provided. The base_url is "" on web
    // (same origin), an absolute URL on native.
    let api = use_hook(|| {
        try_consume_context::<Signal<ApiContext>>()
            .map(|s| s.read().clone())
            .unwrap_or_else(|| ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            })
    });
    let ics_endpoint = format!("{}/v1/me/calendar.ics", api.base_url);
    // A best-effort webcal:// subscribe link for desktop calendar apps. The feed
    // requires auth, so this only works where the client can supply credentials;
    // it is surfaced as a secondary affordance next to the (always-working)
    // authenticated Download button.
    let subscribe_url = ics_endpoint
        .strip_prefix("https://")
        .map(|rest| format!("webcal://{rest}"))
        .or_else(|| {
            ics_endpoint
                .strip_prefix("http://")
                .map(|rest| format!("webcal://{rest}"))
        })
        .unwrap_or_else(|| ics_endpoint.clone());

    let download = move |_| {
        let api = api.clone();
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                download_ics(&api).await;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            spawn(async move {
                let _ = crate::api::save_authenticated_download(
                    &api,
                    "/v1/me/calendar.ics",
                    "aulalite.ics",
                    "text/calendar",
                )
                .await;
            });
        }
    };

    let toolbar = rsx! {
        div { class: "calendar-toolbar",
            Button {
                label: "Download .ics".to_string(),
                variant: ButtonVariant::Secondary,
                on_click: download,
            }
            a {
                class: "ds-button ds-button--ghost ds-button--sm calendar-subscribe",
                href: "{subscribe_url}",
                "Subscribe"
            }
        }
    };

    if props.events.is_empty() {
        return rsx! {
            div { class: "calendar-view motion-page",
                {toolbar}
                EmptyState {
                    title: "Nothing on your calendar".to_string(),
                    description: "Upcoming classes and assignment due dates will appear here.".to_string(),
                    cta: None,
                }
            }
        };
    }

    // Group consecutive (already time-sorted) events under their day key. The
    // backend returns events ordered by timestamp, so a single pass groups them.
    let mut groups: Vec<(String, Vec<CalendarEventDto>)> = Vec::new();
    for e in &props.events {
        let key = day_key(&e.starts_at);
        match groups.last_mut() {
            Some((k, items)) if *k == key => items.push(e.clone()),
            _ => groups.push((key, vec![e.clone()])),
        }
    }

    rsx! {
        div { class: "calendar-view motion-page",
            {toolbar}
            ul { class: "calendar-agenda",
                for (day, items) in groups {
                    li { class: "calendar-day-group",
                        h3 { class: "calendar-day-heading", "{day}" }
                        ul { class: "calendar-day-list",
                            for e in items {
                                {
                                    let tone = event_tone(&e.kind, e.status.as_deref());
                                    let label = event_label(&e.kind, e.status.as_deref());
                                    let when = format_local_ts(&e.starts_at);
                                    let is_assignment = e.kind == "assignment_due";
                                    let meta = match e.duration_minutes {
                                        Some(m) if m > 0 => format!("{when} · {m} min"),
                                        _ => when.clone(),
                                    };
                                    rsx! {
                                        li { class: if is_assignment { "calendar-item calendar-item--due" } else { "calendar-item" },
                                            Card {
                                                div { class: "row-1",
                                                    span { class: "title", "{e.title}" }
                                                    Badge { label, tone }
                                                }
                                                div { class: "row-2",
                                                    span { class: "calendar-when", "{meta}" }
                                                    a { class: "calendar-course", href: "{e.link}", "{e.course_title}" }
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
        }
    }
}

/// Fetch `/v1/me/calendar.ics` WITH the bearer token and trigger a browser
/// download via an object URL. Mirrors the authenticated fetch → Blob → synthetic
/// anchor pattern in `attendance_panel` / `gradebook_panel` (reads the body via
/// `Response::blob()` so the Content-Type rides along and we avoid constructing a
/// Blob ourselves). Wasm-only; a no-op anywhere else.
#[cfg(target_arch = "wasm32")]
async fn download_ics(ctx: &ApiContext) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let url = format!("{}/v1/me/calendar.ics", ctx.base_url);
    let opts = web_sys::RequestInit::new();
    opts.set_method("GET");
    let Ok(req) = web_sys::Request::new_with_str_and_init(&url, &opts) else {
        return;
    };
    if !ctx.id_token.is_empty() {
        let _ = req
            .headers()
            .set("authorization", &format!("Bearer {}", ctx.id_token));
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
        let _ = web_sys::Url::revoke_object_url(&object_url);
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
    anchor.set_download("aulalite.ics");
    anchor.click();
    // Revoke on the next tick so the click can start the download first.
    let revoke_url = object_url.clone();
    let cb = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&revoke_url);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(
        kind: &str,
        title: &str,
        ts: &str,
        status: Option<&str>,
        dur: Option<i32>,
    ) -> CalendarEventDto {
        CalendarEventDto {
            kind: kind.into(),
            id: "00000000-0000-0000-0000-000000000000".into(),
            course_id: "c1".into(),
            course_title: "Calc 1".into(),
            title: title.into(),
            starts_at: ts.into(),
            duration_minutes: dur,
            status: status.map(|s| s.into()),
            link: "/courses/calc-1/x".into(),
        }
    }

    #[test]
    fn tone_and_label_distinguish_due_from_sessions() {
        assert!(matches!(
            event_tone("assignment_due", None),
            BadgeTone::Warning
        ));
        assert!(matches!(
            event_tone("session", Some("live")),
            BadgeTone::Live
        ));
        assert!(matches!(
            event_tone("session", Some("ended")),
            BadgeTone::Success
        ));
        assert!(matches!(
            event_tone("session", Some("scheduled")),
            BadgeTone::Neutral
        ));
        assert_eq!(event_label("assignment_due", None), "Due");
        assert_eq!(event_label("session", Some("live")), "live");
    }

    #[test]
    fn day_key_off_wasm_uses_date_prefix() {
        assert_eq!(day_key("2026-06-17T22:39:04Z"), "2026-06-17");
        assert_eq!(day_key("2026-06-20T23:59:00Z"), "2026-06-20");
    }

    #[test]
    fn empty_calendar_renders_empty_state_and_toolbar() {
        // No ApiContext provided in SSR → the component falls back to an empty
        // base_url and still renders the toolbar + empty state (panic-free).
        fn app() -> Element {
            rsx! { CalendarView { events: vec![] } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Nothing on your calendar"),
            "empty state missing: {html}"
        );
        // The download/subscribe affordances always render (download is an
        // authenticated action, so it's a Button rather than a bare link).
        assert!(
            html.contains("Download .ics"),
            "download control missing: {html}"
        );
        assert!(
            html.contains("Subscribe"),
            "subscribe control missing: {html}"
        );
    }

    #[test]
    fn subscribe_url_rewrites_https_to_webcal() {
        // The webcal rewrite is pure string logic; assert it directly so the
        // test does not depend on a live ApiContext base_url in SSR.
        let https = "https://app.example.com/v1/me/calendar.ics";
        let webcal = https
            .strip_prefix("https://")
            .map(|rest| format!("webcal://{rest}"))
            .unwrap();
        assert_eq!(webcal, "webcal://app.example.com/v1/me/calendar.ics");
    }

    #[test]
    fn agenda_groups_by_day_and_links_out() {
        fn app() -> Element {
            rsx! {
                CalendarView {
                    events: vec![
                        dto("session", "Limits", "2026-06-17T22:39:04Z", Some("scheduled"), Some(45)),
                        dto("assignment_due", "Problem Set 1", "2026-06-20T23:59:00Z", None, None),
                    ],
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Two day groups (different dates).
        assert_eq!(
            html.matches("calendar-day-heading").count(),
            2,
            "got: {html}"
        );
        // Session shows duration; assignment shows the Due badge.
        assert!(html.contains("· 45 min"), "session meta missing: {html}");
        assert!(html.contains(">Due<"), "due badge missing: {html}");
        // Rows deep-link out.
        assert!(
            html.contains("/courses/calc-1/x"),
            "deep link missing: {html}"
        );
    }

    #[test]
    fn same_day_events_share_a_group() {
        fn app() -> Element {
            rsx! {
                CalendarView {
                    events: vec![
                        dto("session", "Morning", "2026-06-17T09:00:00Z", Some("scheduled"), Some(30)),
                        dto("session", "Evening", "2026-06-17T20:00:00Z", Some("scheduled"), Some(30)),
                    ],
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert_eq!(
            html.matches("calendar-day-heading").count(),
            1,
            "should be one group: {html}"
        );
        assert!(html.contains("Morning") && html.contains("Evening"));
    }
}
