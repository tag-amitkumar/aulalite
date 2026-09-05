// crates/shell-web/src/routes/parent_home.rs
//
// Parent / guardian dashboard. Restricted to users whose tenant role is Parent
// (platform admins in an explicitly selected workspace are also admitted for
// support). The page lists the children linked to
// the signed-in parent, lets them pick one, and surfaces that child's released
// grades, attendance rollup, and upcoming schedule as kinetics MetricCards and
// DataTables.
//
// Every backend endpoint here is read-only and 403s for an unlinked child, so
// each section follows the project-wide four-state UX (loading skeletons /
// error / empty / loaded).
use core_types::Capability;
use design_system::kinetics_ui::{
    DataTable, DataTableColumn, DataTableRow, MetricCard, MetricTone, StreakBadge,
};
use design_system::{EmptyState, PageHeader, Select, SelectOption, SkeletonCard, SkeletonLine};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::{
    ChildAttendanceDto, ChildDto, ChildGamificationDto, ChildGradeDto, ChildScheduleItemDto,
};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// localStorage key for the last-viewed child, so parents with several
/// children land on the one they were looking at last time.
const SELECTED_CHILD_KEY: &str = "aula.parent-selected-child";

fn persisted_child() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(SELECTED_CHILD_KEY).ok().flatten())
            .filter(|v| !v.is_empty())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

fn persist_child(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item(SELECTED_CHILD_KEY, id);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

/// The displayed label for a child: display_name, else email, else a short
/// slice of the user id so a row/option is never blank.
fn child_label(c: &ChildDto) -> String {
    if let Some(name) = c.display_name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = c.email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    c.student_user_id.chars().take(8).collect()
}

/// Format an RFC3339 timestamp as `MMM D, YYYY h:mm AM/PM`, falling back to the
/// raw string when parsing fails.
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Format a duration in seconds as `Xm Ys` (e.g. 605 → "10m 5s").
fn format_secs_ms(total_seconds: i64) -> String {
    let secs = total_seconds.max(0);
    format!("{}m {}s", secs / 60, secs % 60)
}

/// Format a (potentially large) duration in seconds as `Xh Ym` for the metric
/// tile (e.g. 3_660 → "1h 1m").
fn format_secs_hm(total_seconds: i64) -> String {
    let secs = total_seconds.max(0);
    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
}

/// The grade display for one assignment: numeric (1-decimal), else letter, else
/// pass/fail, else the raw status as a fallback.
fn grade_display(g: &ChildGradeDto) -> String {
    if let Some(n) = g.numeric_grade {
        return format!("{n:.1}");
    }
    if let Some(letter) = g.letter_grade.as_ref().filter(|s| !s.is_empty()) {
        return letter.clone();
    }
    match g.passed {
        Some(true) => "Pass".to_string(),
        Some(false) => "Fail".to_string(),
        None => g.status.clone(),
    }
}

/// The child's XP/level/streak chips row. Renders nothing when the child has
/// no XP yet (fresh accounts, courses without gamified activity). Pure so it's
/// SSR-testable.
fn gamify_chips(g: &ChildGamificationDto) -> Element {
    if g.total_xp == 0 {
        return rsx! {};
    }
    rsx! {
        div { class: "gamify-strip parent-gamify-strip",
            MetricCard {
                label: "Level".to_string(),
                value: g.level.to_string(),
                delta: format!("{} XP total", g.total_xp),
                tone: MetricTone::Info,
            }
            div { class: "gamify-streak-chip",
                StreakBadge {
                    days: g.current_streak_days.max(0) as u32,
                    active: g.streak_active_today,
                }
                span { class: "gamify-streak-caption muted", "day streak" }
            }
        }
    }
}

/// Compute the metric tiles for a child from the three loaded datasets. Pure so
/// it's SSR-testable.
fn metric_grid(grades: &[ChildGradeDto], attendance: &[ChildAttendanceDto]) -> Element {
    let released = grades.iter().filter(|g| g.released_at.is_some()).count();

    let numerics: Vec<f64> = grades.iter().filter_map(|g| g.numeric_grade).collect();
    let avg_value = if numerics.is_empty() {
        "—".to_string()
    } else {
        let avg = numerics.iter().sum::<f64>() / numerics.len() as f64;
        format!("{avg:.1}")
    };

    let sessions_attended = attendance.len();
    let total_seconds: i64 = attendance.iter().map(|a| a.total_seconds).sum();

    rsx! {
        div { class: "parent-metric-grid admin-analytics-grid",
            MetricCard {
                label: "Released grades".to_string(),
                value: released.to_string(),
                tone: MetricTone::Info,
            }
            MetricCard {
                label: "Avg grade".to_string(),
                value: avg_value,
                delta: format!("{} numeric", numerics.len()),
                tone: MetricTone::Neutral,
            }
            MetricCard {
                label: "Sessions attended".to_string(),
                value: sessions_attended.to_string(),
                tone: MetricTone::Success,
            }
            MetricCard {
                label: "Attendance time".to_string(),
                value: format_secs_hm(total_seconds),
                tone: MetricTone::Neutral,
            }
        }
    }
}

/// Grades DataTable. Pure so it's SSR-testable.
fn grades_table(grades: &[ChildGradeDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("course", "Course"),
        DataTableColumn::new("assignment", "Assignment"),
        DataTableColumn::new("grade", "Grade"),
        DataTableColumn::new("released", "Released"),
    ];
    let rows = grades
        .iter()
        .map(|g| {
            let course = g
                .course_title
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".to_string());
            let released = g
                .released_at
                .as_deref()
                .map(format_ts)
                .unwrap_or_else(|| "—".to_string());
            DataTableRow::new(
                g.assignment_id.clone(),
                vec![
                    course,
                    g.assignment_title.clone(),
                    grade_display(g),
                    released,
                ],
            )
        })
        .collect::<Vec<_>>();
    rsx! {
        // The visible "Grades" h2 already titles this table; the caption is
        // screen-reader context only (visually hidden via CSS).
        DataTable { columns, rows, caption: "Released grades for the selected child" }
    }
}

/// Attendance DataTable. Pure so it's SSR-testable.
fn attendance_table(attendance: &[ChildAttendanceDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("session", "Session"),
        DataTableColumn::new("course", "Course"),
        DataTableColumn::new("joined", "Joined"),
        DataTableColumn::new("time", "Time"),
        DataTableColumn::new("reconnects", "Reconnects"),
    ];
    let rows = attendance
        .iter()
        .map(|a| {
            let session = a
                .session_title
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Live session".to_string());
            let course = a
                .course_title
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".to_string());
            let joined = format_ts(&a.first_joined_at);
            let time = format_secs_ms(a.total_seconds);
            let reconnects = a.reconnect_count.to_string();
            DataTableRow::new(
                a.session_id.clone(),
                vec![session, course, joined, time, reconnects],
            )
        })
        .collect::<Vec<_>>();
    rsx! {
        // Captioned for screen readers; the "Attendance" h2 is the visible title.
        DataTable { columns, rows, caption: "Live-session attendance for the selected child" }
    }
}

/// Upcoming-schedule DataTable. Pure so it's SSR-testable.
fn schedule_table(schedule: &[ChildScheduleItemDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("course", "Course"),
        DataTableColumn::new("session", "Session"),
        DataTableColumn::new("starts", "Starts"),
        DataTableColumn::new("duration", "Duration"),
    ];
    let rows = schedule
        .iter()
        .map(|s| {
            DataTableRow::new(
                s.session_id.clone(),
                vec![
                    s.course_title.clone(),
                    s.title.clone(),
                    format_ts(&s.starts_at),
                    format!("{} min", s.duration_minutes),
                ],
            )
        })
        .collect::<Vec<_>>();
    rsx! {
        // Captioned for screen readers; the "Upcoming schedule" h2 is the
        // visible title.
        DataTable { columns, rows, caption: "Upcoming sessions for the selected child" }
    }
}

/// Two-line skeleton placeholder used while a section loads.
fn section_loading() -> Element {
    rsx! {
        div { class: "system-state system-state--loading",
            SkeletonLine { width: "70%".to_string() }
            SkeletonLine { width: "55%".to_string() }
        }
    }
}

#[component]
pub fn ParentHome() -> Element {
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

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    // Gate: only parents (or platform admins, gently) reach the dashboard.
    if !user.has_capability(Capability::ParentRead) {
        let body = rsx! {
            div { class: "parent-dashboard-page",
                PageHeader {
                    title: "Parent dashboard".to_string(),
                    kicker: "Family".to_string(),
                }
                div { class: "system-state",
                    p { "This area is for parent/guardian accounts." }
                }
            }
        };
        return rsx! {
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
        };
    }

    let children = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_parent_children(&api).await }
        }
    });

    // The selected child id. Defaults to the first child once the list loads.
    let mut selected = use_signal(String::new);

    let children_snap = children.read_unchecked();

    // Seed the selection once the list resolves: the persisted last-viewed
    // child when still linked, else the first child.
    if selected.read().is_empty() {
        if let Some(Ok(list)) = children_snap.as_ref() {
            let remembered =
                persisted_child().filter(|id| list.iter().any(|c| c.student_user_id == *id));
            if let Some(id) = remembered {
                selected.set(id);
            } else if let Some(first) = list.first() {
                selected.set(first.student_user_id.clone());
            }
        }
    }

    let body: Element = match children_snap.as_ref() {
        None => rsx! {
            div { class: "parent-dashboard-page",
                PageHeader {
                    title: "Parent dashboard".to_string(),
                    kicker: "Family".to_string(),
                    subtitle: "Grades, attendance, and upcoming sessions for your children.".to_string(),
                }
                { section_loading() }
            }
        },
        Some(Err(e)) => rsx! {
            div { class: "parent-dashboard-page",
                PageHeader {
                    title: "Parent dashboard".to_string(),
                    kicker: "Family".to_string(),
                }
                p { class: "error", "Could not load your children: {e}" }
            }
        },
        Some(Ok(list)) if list.is_empty() => rsx! {
            div { class: "parent-dashboard-page",
                PageHeader {
                    title: "Parent dashboard".to_string(),
                    kicker: "Family".to_string(),
                }
                EmptyState {
                    title: "No linked children yet.".to_string(),
                    description: "Once a school administrator links a student to your account, their grades, attendance, and schedule will appear here.".to_string(),
                }
            }
        },
        Some(Ok(list)) => {
            let options: Vec<SelectOption> = list
                .iter()
                .map(|c| SelectOption {
                    value: c.student_user_id.clone(),
                    label: child_label(c),
                })
                .collect();
            let current = selected.read().clone();
            let selected_name = list
                .iter()
                .find(|c| c.student_user_id == current)
                .map(child_label)
                .unwrap_or_default();

            rsx! {
                div { class: "parent-dashboard-page",
                    PageHeader {
                        title: "Parent dashboard".to_string(),
                        kicker: "Family".to_string(),
                        subtitle: "Grades, attendance, and upcoming sessions for your children.".to_string(),
                    }

                    div { class: "parent-child-picker",
                        label { class: "parent-child-picker-label", "Child" }
                        Select {
                            value: current.clone(),
                            options,
                            on_change: move |id: String| {
                                persist_child(&id);
                                selected.set(id);
                            },
                        }
                    }

                    if current.is_empty() {
                        p { class: "muted", "Select a child to view their details." }
                    } else {
                        // Key on the student id so switching children remounts
                        // ChildDetail and re-runs its three resources.
                        ChildDetail {
                            key: "{current}",
                            student_id: current.clone(),
                            child_name: selected_name,
                        }
                    }
                }
            }
        }
    };
    drop(children_snap);

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

#[derive(Props, Clone, PartialEq)]
struct ChildDetailProps {
    student_id: String,
    child_name: String,
}

/// Per-child detail: metric tiles + grades / attendance / schedule tables.
/// Split into its own component (keyed on student_id by the caller via the
/// changing props) so the three resources re-fetch when the selection changes.
#[component]
fn ChildDetail(props: ChildDetailProps) -> Element {
    let api = use_api();
    let student_id = props.student_id.clone();

    let grades = use_resource({
        let api = api.clone();
        let sid = student_id.clone();
        move || {
            let api = api.clone();
            let sid = sid.clone();
            async move { api::get_child_grades(&api, &sid).await }
        }
    });
    let attendance = use_resource({
        let api = api.clone();
        let sid = student_id.clone();
        move || {
            let api = api.clone();
            let sid = sid.clone();
            async move { api::get_child_attendance(&api, &sid).await }
        }
    });
    let schedule = use_resource({
        let api = api.clone();
        let sid = student_id.clone();
        move || {
            let api = api.clone();
            let sid = sid.clone();
            async move { api::get_child_schedule(&api, &sid).await }
        }
    });
    let gamification = use_resource({
        let api = api.clone();
        let sid = student_id.clone();
        move || {
            let api = api.clone();
            let sid = sid.clone();
            async move { api::get_child_gamification(&api, &sid).await }
        }
    });

    let grades_snap = grades.read_unchecked();
    let attendance_snap = attendance.read_unchecked();
    let schedule_snap = schedule.read_unchecked();

    // Metric tiles need both grades + attendance loaded; show a skeleton row
    // until both resolve, an error if either failed, otherwise the grid.
    let metrics: Element = match (grades_snap.as_ref(), attendance_snap.as_ref()) {
        (Some(Ok(g)), Some(Ok(a))) => metric_grid(g, a),
        (Some(Err(e)), _) | (_, Some(Err(e))) => rsx! {
            p { class: "error", "Could not load summary: {e}" }
        },
        _ => rsx! {
            div { class: "parent-metric-grid admin-analytics-grid",
                for _ in 0..4 {
                    SkeletonCard { height: "100px".to_string() }
                }
            }
        },
    };

    let grades_section: Element = match grades_snap.as_ref() {
        None => section_loading(),
        Some(Err(e)) => rsx! { p { class: "error", "Could not load grades: {e}" } },
        Some(Ok(g)) if g.is_empty() => rsx! {
            p { class: "muted", "No released grades yet." }
        },
        Some(Ok(g)) => grades_table(g),
    };

    let attendance_section: Element = match attendance_snap.as_ref() {
        None => section_loading(),
        Some(Err(e)) => rsx! { p { class: "error", "Could not load attendance: {e}" } },
        Some(Ok(a)) if a.is_empty() => rsx! {
            p { class: "muted", "No attendance recorded yet." }
        },
        Some(Ok(a)) => attendance_table(a),
    };

    let schedule_section: Element = match schedule_snap.as_ref() {
        None => section_loading(),
        Some(Err(e)) => rsx! { p { class: "error", "Could not load schedule: {e}" } },
        Some(Ok(s)) if s.is_empty() => rsx! {
            p { class: "muted", "No upcoming sessions in the next 30 days." }
        },
        Some(Ok(s)) => schedule_table(s),
    };

    // XP/streak chips are decorative — loading and error states just hide them.
    let gamify_snap = gamification.read_unchecked();
    let gamify_section: Element = match gamify_snap.as_ref() {
        Some(Ok(g)) => gamify_chips(g),
        _ => rsx! {},
    };
    drop(gamify_snap);

    let name = props.child_name.clone();

    rsx! {
        section { class: "parent-child-detail",
            { gamify_section }
            { metrics }

            h2 { class: "parent-section-title", "Grades" }
            { grades_section }

            h2 { class: "parent-section-title", "Attendance" }
            { attendance_section }

            h2 { class: "parent-section-title", "Upcoming schedule" }
            { schedule_section }

            if !name.is_empty() {
                p { class: "visually-hidden", "Details for {name}" }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    fn grade(id: &str, numeric: Option<f64>, released: bool) -> ChildGradeDto {
        ChildGradeDto {
            assignment_id: id.into(),
            assignment_title: "Essay".into(),
            course_title: Some("History".into()),
            status: "graded".into(),
            numeric_grade: numeric,
            letter_grade: None,
            passed: None,
            student_visible_feedback: None,
            released_at: if released {
                Some("2026-05-01T10:00:00Z".into())
            } else {
                None
            },
        }
    }

    fn attendance(id: &str, secs: i64) -> ChildAttendanceDto {
        ChildAttendanceDto {
            session_id: id.into(),
            session_title: Some("Lecture 1".into()),
            course_title: Some("History".into()),
            first_joined_at: "2026-05-01T09:00:00Z".into(),
            last_left_at: None,
            total_seconds: secs,
            reconnect_count: 1,
            starts_at: None,
        }
    }

    #[test]
    fn grade_display_prefers_numeric_then_letter_then_passfail() {
        assert_eq!(grade_display(&grade("a", Some(87.25), true)), "87.2");
        let mut letter = grade("b", None, true);
        letter.letter_grade = Some("A-".into());
        assert_eq!(grade_display(&letter), "A-");
        let mut pass = grade("c", None, true);
        pass.passed = Some(true);
        assert_eq!(grade_display(&pass), "Pass");
        let mut fail = grade("d", None, true);
        fail.passed = Some(false);
        assert_eq!(grade_display(&fail), "Fail");
    }

    #[test]
    fn duration_formatters() {
        assert_eq!(format_secs_ms(605), "10m 5s");
        assert_eq!(format_secs_ms(-5), "0m 0s");
        assert_eq!(format_secs_hm(3_660), "1h 1m");
        assert_eq!(format_secs_hm(0), "0h 0m");
    }

    #[test]
    fn child_label_prefers_name_then_email_then_short_id() {
        let c = ChildDto {
            student_user_id: "0123456789abcdef".into(),
            display_name: Some("Sam".into()),
            email: Some("sam@x".into()),
        };
        assert_eq!(child_label(&c), "Sam");
        let c2 = ChildDto {
            display_name: None,
            ..c.clone()
        };
        assert_eq!(child_label(&c2), "sam@x");
        let c3 = ChildDto {
            display_name: None,
            email: None,
            ..c.clone()
        };
        assert_eq!(child_label(&c3), "01234567");
    }

    #[test]
    fn gamify_chips_render_level_and_streak_but_hide_at_zero_xp() {
        fn app_inner(g: ChildGamificationDto) -> Element {
            gamify_chips(&g)
        }
        let g = ChildGamificationDto {
            total_xp: 250,
            level: 2,
            current_streak_days: 3,
            longest_streak_days: 5,
            streak_active_today: true,
        };
        let mut vdom = VirtualDom::new_with_props(app_inner, g.clone());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Level"), "got: {html}");
        assert!(html.contains("250 XP total"), "got: {html}");
        assert!(
            html.contains("ui-streak-badge"),
            "streak badge missing: {html}"
        );
        assert!(
            html.contains("day streak"),
            "streak caption missing: {html}"
        );

        let zero = ChildGamificationDto { total_xp: 0, ..g };
        let mut vdom = VirtualDom::new_with_props(app_inner, zero);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("Level"), "should hide at 0 XP: {html}");
    }

    #[test]
    fn metric_grid_renders_cards() {
        fn app_inner(args: (Vec<ChildGradeDto>, Vec<ChildAttendanceDto>)) -> Element {
            metric_grid(&args.0, &args.1)
        }
        let grades = vec![grade("a", Some(80.0), true), grade("b", Some(90.0), true)];
        let att = vec![attendance("s1", 3_600), attendance("s2", 1_800)];
        let mut vdom = VirtualDom::new_with_props(app_inner, (grades, att));
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-metric-card"), "got: {html}");
        assert!(html.contains("Released grades"));
        assert!(html.contains("Avg grade"));
        // Avg of 80 and 90 → 85.0
        assert!(html.contains("85.0"), "avg missing: {html}");
        // 3600 + 1800 = 5400s → 1h 30m
        assert!(html.contains("1h 30m"), "attendance time missing: {html}");
    }

    #[test]
    fn grades_table_renders_data_table() {
        fn app_inner(g: Vec<ChildGradeDto>) -> Element {
            grades_table(&g)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, vec![grade("a", Some(80.0), true)]);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        assert!(html.contains("History"));
        assert!(html.contains("Essay"));
        assert!(html.contains("80.0"));
        // Caption is screen-reader context, distinct from the visible h2.
        assert!(
            html.contains("Released grades for the selected child"),
            "got: {html}"
        );
    }

    #[test]
    fn empty_state_renders_for_no_children() {
        // Mirrors the no-children branch of ParentHome's body.
        fn app() -> Element {
            rsx! {
                EmptyState {
                    title: "No linked children yet.".to_string(),
                    description: "desc".to_string(),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("No linked children yet."), "got: {html}");
        assert!(html.contains("empty-state-title"));
    }

    #[test]
    fn page_header_renders_title() {
        fn app() -> Element {
            rsx! {
                PageHeader {
                    title: "Parent dashboard".to_string(),
                    kicker: "Family".to_string(),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Parent dashboard"), "got: {html}");
    }
}
