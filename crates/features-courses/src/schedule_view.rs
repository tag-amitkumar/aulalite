// crates/features-courses/src/schedule_view.rs
use design_system::{Badge, BadgeTone, Button, ButtonVariant, Card, EmptyState};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct ScheduleEntry {
    pub session_id: String,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at_display: String,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
    pub can_edit: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct ScheduleViewProps {
    pub entries: Vec<ScheduleEntry>,
    pub on_cancel: EventHandler<String>,
    pub on_reschedule: EventHandler<String>,
}

/// Format a raw session timestamp as e.g. "Jun 17, 2026 10:39 PM", falling
/// back to the raw string when parsing fails (same pattern as shell-web's
/// `parent_home::format_ts`). Backend timestamps sometimes arrive without a
/// zone (e.g. "2026-06-17T22:39:04.5996892"), which is not valid RFC3339, so
/// we also try a naive-datetime parse before giving up.
pub(crate) fn format_session_ts(raw: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return dt.format("%b %-d, %Y %-I:%M %p").to_string();
    }
    raw.parse::<chrono::NaiveDateTime>()
        .map(|dt| dt.format("%b %-d, %Y %-I:%M %p").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// Derive the [`BadgeTone`] for a schedule entry from its status + diverged flag.
///
/// - `live` → `Live` (animated pulse)
/// - `completed` / `ended` → `Success`
/// - `cancelled` → `Danger`
/// - `scheduled` with `diverged = true` → `Warning` (edited from series)
/// - anything else → `Neutral`
fn entry_tone(status: &str, diverged: bool) -> BadgeTone {
    match (status, diverged) {
        ("live", _) => BadgeTone::Live,
        ("completed", _) | ("ended", _) => BadgeTone::Success,
        ("cancelled", _) => BadgeTone::Danger,
        ("scheduled", true) => BadgeTone::Warning,
        _ => BadgeTone::Neutral,
    }
}

#[component]
pub fn ScheduleView(props: ScheduleViewProps) -> Element {
    if props.entries.is_empty() {
        return rsx! {
            EmptyState {
                title: "No sessions scheduled".to_string(),
                description: "When a teacher schedules a class, you'll see it here.".to_string(),
                cta: None,
            }
        };
    }
    rsx! {
        div { class: "schedule-agenda motion-page",
            ul { class: "schedule-list",
                for e in &props.entries {
                    {
                        let session_id = e.session_id.clone();
                        let cancel_id = session_id.clone();
                        let resched_id = session_id.clone();
                        let on_cancel = props.on_cancel;
                        let on_resched = props.on_reschedule;
                        let cancelled = e.status == "cancelled";
                        let tone = entry_tone(&e.status, e.diverged);
                        let starts_at = format_session_ts(&e.starts_at_display);
                        // Rows shouldn't dead-end at the course page: a live
                        // session gets a loud "Join now →", a scheduled one a
                        // quiet "Details" link to the same session room.
                        let session_href =
                            format!("/courses/{}/sessions/{}", e.course_slug, e.session_id);
                        rsx! {
                            li { class: if cancelled { "schedule-item cancelled" } else { "schedule-item" },
                                Card {
                                    div { class: "row-1",
                                        span { class: "title", "{e.title}" }
                                        Badge { label: e.status.clone(), tone }
                                        if e.diverged && !cancelled {
                                            Badge { label: "edited".to_string(), tone: BadgeTone::Warning }
                                        }
                                    }
                                    div { class: "row-2",
                                        span { "{starts_at} · {e.duration_minutes} min" }
                                        a { href: "/courses/{e.course_slug}", "{e.course_title}" }
                                        if e.status == "live" {
                                            a {
                                                class: "ds-button ds-button--primary ds-button--sm schedule-join-now",
                                                href: "{session_href}",
                                                "Join now →"
                                            }
                                        } else if e.status == "scheduled" {
                                            a {
                                                class: "ds-button ds-button--ghost ds-button--sm schedule-session-details",
                                                href: "{session_href}",
                                                "Details"
                                            }
                                        }
                                    }
                                    if e.can_edit && !cancelled {
                                        div { class: "row-3 actions",
                                            // Ghost, not Danger: cancelling is a secondary action
                                            // and shouldn't be the loudest element of the row.
                                            Button {
                                                label: "Cancel".to_string(),
                                                variant: ButtonVariant::Ghost,
                                                on_click: move |_| on_cancel.call(cancel_id.clone()),
                                            }
                                            Button {
                                                label: "Reschedule".to_string(),
                                                variant: ButtonVariant::Link,
                                                on_click: move |_| on_resched.call(resched_id.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_live_takes_priority() {
        assert!(matches!(entry_tone("live", false), BadgeTone::Live));
        assert!(matches!(entry_tone("live", true), BadgeTone::Live));
    }

    #[test]
    fn tone_completed_and_ended_are_success() {
        assert!(matches!(entry_tone("completed", false), BadgeTone::Success));
        assert!(matches!(entry_tone("ended", false), BadgeTone::Success));
    }

    #[test]
    fn tone_cancelled_is_danger() {
        assert!(matches!(entry_tone("cancelled", false), BadgeTone::Danger));
        assert!(matches!(entry_tone("cancelled", true), BadgeTone::Danger));
    }

    #[test]
    fn tone_scheduled_diverged_is_warning() {
        assert!(matches!(entry_tone("scheduled", true), BadgeTone::Warning));
        assert!(matches!(entry_tone("scheduled", false), BadgeTone::Neutral));
    }

    #[test]
    fn tone_unknown_status_is_neutral() {
        assert!(matches!(entry_tone("queued", false), BadgeTone::Neutral));
    }

    #[test]
    fn format_session_ts_humanizes_rfc3339_and_naive() {
        assert_eq!(
            format_session_ts("2026-06-17T22:39:04Z"),
            "Jun 17, 2026 10:39 PM"
        );
        // Zone-less backend timestamps (not valid RFC3339) still humanize.
        assert_eq!(
            format_session_ts("2026-06-17T22:39:04.5996892"),
            "Jun 17, 2026 10:39 PM"
        );
        // Unparseable input falls back to the raw string.
        assert_eq!(format_session_ts("soon"), "soon");
    }

    #[test]
    fn row_renders_humanized_time_and_ghost_cancel() {
        fn app() -> Element {
            rsx! {
                ScheduleView {
                    entries: vec![ScheduleEntry {
                        session_id: "s1".into(),
                        course_title: "Calc 1".into(),
                        course_slug: "calc-1".into(),
                        title: "Limits".into(),
                        starts_at_display: "2026-06-17T22:39:04.5996892".into(),
                        duration_minutes: 45,
                        status: "scheduled".into(),
                        diverged: false,
                        can_edit: true,
                    }],
                    on_cancel: |_| {},
                    on_reschedule: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Jun 17, 2026 10:39 PM · 45 min"),
            "humanized time missing: {html}"
        );
        assert!(
            !html.contains("2026-06-17T22:39"),
            "raw ISO timestamp leaked: {html}"
        );
        // Cancel is demoted to the ghost variant (no red Danger button).
        assert!(
            html.contains("ds-button--ghost"),
            "ghost cancel missing: {html}"
        );
        assert!(
            !html.contains("ds-button--destructive"),
            "destructive cancel leaked: {html}"
        );
    }

    #[test]
    fn live_row_renders_join_now_link() {
        fn app() -> Element {
            rsx! {
                ScheduleView {
                    entries: vec![ScheduleEntry {
                        session_id: "s1".into(),
                        course_title: "Calc 1".into(),
                        course_slug: "calc-1".into(),
                        title: "Limits".into(),
                        starts_at_display: "2026-06-17T22:39:04Z".into(),
                        duration_minutes: 45,
                        status: "live".into(),
                        diverged: false,
                        can_edit: false,
                    }],
                    on_cancel: |_| {},
                    on_reschedule: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Join now →"), "join CTA missing: {html}");
        assert!(
            html.contains("/courses/calc-1/sessions/s1"),
            "session href missing: {html}"
        );
        assert!(
            html.contains("schedule-join-now"),
            "join CTA class missing: {html}"
        );
        assert!(
            !html.contains("Details"),
            "quiet link leaked on live row: {html}"
        );
    }

    #[test]
    fn scheduled_row_renders_quiet_details_link() {
        fn app() -> Element {
            rsx! {
                ScheduleView {
                    entries: vec![ScheduleEntry {
                        session_id: "s2".into(),
                        course_title: "Calc 1".into(),
                        course_slug: "calc-1".into(),
                        title: "Derivatives".into(),
                        starts_at_display: "2026-06-18T22:39:04Z".into(),
                        duration_minutes: 45,
                        status: "scheduled".into(),
                        diverged: false,
                        can_edit: false,
                    }],
                    on_cancel: |_| {},
                    on_reschedule: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Details"), "details link missing: {html}");
        assert!(
            html.contains("/courses/calc-1/sessions/s2"),
            "session href missing: {html}"
        );
        assert!(
            !html.contains("Join now"),
            "loud join CTA leaked on scheduled row: {html}"
        );
    }

    #[test]
    fn cancelled_row_has_no_session_link() {
        fn app() -> Element {
            rsx! {
                ScheduleView {
                    entries: vec![ScheduleEntry {
                        session_id: "s3".into(),
                        course_title: "Calc 1".into(),
                        course_slug: "calc-1".into(),
                        title: "Integrals".into(),
                        starts_at_display: "2026-06-19T22:39:04Z".into(),
                        duration_minutes: 45,
                        status: "cancelled".into(),
                        diverged: false,
                        can_edit: false,
                    }],
                    on_cancel: |_| {},
                    on_reschedule: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("/courses/calc-1/sessions/s3"),
            "session link leaked on cancelled row: {html}"
        );
    }
}
