// crates/shell-web/src/routes/admin_analytics.rs
//
// Org-wide analytics dashboard. Restricted to org-admin / platform-admin users
// (the backend `/v1/analytics/overview` endpoint 403s everyone else). Renders a
// PageHeader plus a responsive grid of kinetics `MetricCard` tiles summarizing
// the tenant. Follows the project-wide four-state UX: loading skeletons,
// error, (no empty state — the overview always returns numbers), and loaded.
use design_system::kinetics_ui::{
    ChartSeries, ChartTone, DataTable, DataTableColumn, DataTableRow, DonutGauge, LineChart,
    MetricCard, MetricTone,
};
use design_system::{Card, PageHeader, SkeletonCard};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::{DailyActivityDto, OverviewDto};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Build the grid of metric tiles from the overview rollup. Pure so it's
/// SSR-testable without a live ApiContext.
fn overview_grid(o: &OverviewDto) -> Element {
    rsx! {
        div { class: "admin-analytics-grid",
            MetricCard {
                label: "Courses".to_string(),
                value: o.courses_total.to_string(),
                delta: format!("{} published", o.courses_published),
                tone: MetricTone::Neutral,
            }
            MetricCard {
                label: "Students".to_string(),
                value: o.members_students.to_string(),
                tone: MetricTone::Info,
            }
            MetricCard {
                label: "Teachers".to_string(),
                value: o.members_teachers.to_string(),
                delta: format!("{} TAs", o.members_tas),
                tone: MetricTone::Info,
            }
            MetricCard {
                label: "Sessions (30d)".to_string(),
                value: o.sessions_last_30d.to_string(),
                delta: format!("{} ended all-time", o.sessions_ended_total),
                tone: MetricTone::Success,
            }
            MetricCard {
                label: "Recordings".to_string(),
                value: o.recordings_available.to_string(),
                tone: MetricTone::Neutral,
            }
            MetricCard {
                label: "Assignments".to_string(),
                value: o.assignments_total.to_string(),
                tone: MetricTone::Neutral,
            }
            MetricCard {
                label: "Submissions".to_string(),
                value: o.submissions_total.to_string(),
                delta: format!("{} graded", o.submissions_graded),
                tone: MetricTone::Warning,
            }
        }
    }
}

/// Sparse x-axis labels for a daily series: first, last, and every 7th day as
/// `MM-DD`, everything else empty so the axis stays readable at 30 points.
fn sparse_day_labels(rows: &[DailyActivityDto]) -> Vec<String> {
    let n = rows.len();
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            if i == 0 || i + 1 == n || i % 7 == 0 {
                r.day.get(5..).unwrap_or(&r.day).to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

/// The engagement-over-time chart (primary) with its accessible data table in
/// a "View data" expander. Pure so it's SSR-testable.
fn activity_section(rows: &[DailyActivityDto]) -> Element {
    let series = vec![
        ChartSeries::new(
            "Attendance joins",
            rows.iter().map(|r| r.attendance_joins as f32).collect(),
        ),
        ChartSeries::new(
            "Lessons completed",
            rows.iter().map(|r| r.lessons_completed as f32).collect(),
        ),
        ChartSeries::new(
            "Submissions",
            rows.iter().map(|r| r.submissions as f32).collect(),
        ),
    ];
    let x_labels = sparse_day_labels(rows);

    let columns = vec![
        DataTableColumn::new("day", "Day"),
        DataTableColumn::new("sessions", "Sessions"),
        DataTableColumn::new("attendance", "Attendance joins"),
        DataTableColumn::new("lessons", "Lessons completed"),
        DataTableColumn::new("submissions", "Submissions"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|r| {
            DataTableRow::new(
                r.day.clone(),
                vec![
                    r.day.clone(),
                    r.sessions.to_string(),
                    r.attendance_joins.to_string(),
                    r.lessons_completed.to_string(),
                    r.submissions.to_string(),
                ],
            )
        })
        .collect();

    rsx! {
        Card {
            // Clamp the chart's rendered height (the SVG otherwise scales to
            // the full card width and gets ~700px tall on wide screens).
            div { class: "analytics-chart-clamp",
                LineChart {
                    label: "Engagement — last 30 days".to_string(),
                    series,
                    x_labels,
                    show_area: true,
                }
            }
            details { class: "chart-data-expander",
                summary { "View data" }
                DataTable {
                    columns,
                    rows: table_rows,
                    caption: "Daily activity".to_string(),
                }
            }
        }
    }
}

/// Coverage gauges derived from the overview rollup. Pure so it's SSR-testable.
fn gauges_row(o: &OverviewDto) -> Element {
    let grading = if o.submissions_total > 0 {
        o.submissions_graded as f32 / o.submissions_total as f32
    } else {
        0.0
    };
    let published = if o.courses_total > 0 {
        o.courses_published as f32 / o.courses_total as f32
    } else {
        0.0
    };
    rsx! {
        div { class: "analytics-gauges-row",
            DonutGauge {
                label: "Grading coverage".to_string(),
                value: grading,
                description: format!("{} of {} submissions graded", o.submissions_graded, o.submissions_total),
                tone: ChartTone::Success,
            }
            DonutGauge {
                label: "Courses published".to_string(),
                value: published,
                description: format!("{} of {} courses live", o.courses_published, o.courses_total),
                tone: ChartTone::Info,
            }
        }
    }
}

/// Skeleton placeholder grid shown while the overview loads.
fn loading_grid() -> Element {
    rsx! {
        div { class: "admin-analytics-grid",
            for _ in 0..7 {
                SkeletonCard { height: "120px".to_string() }
            }
        }
    }
}

#[component]
pub fn AdminAnalytics() -> Element {
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
                p { "You don't have permission to view analytics." }
            }
        };
    }

    let overview = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_analytics_overview(&api).await }
        }
    });
    let activity = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_analytics_activity(&api, 30).await }
        }
    });

    let snap = overview.read_unchecked();
    let grid: Element = match snap.as_ref() {
        Some(Ok(o)) => overview_grid(o),
        Some(Err(e)) => rsx! {
            p { class: "error", "Could not load analytics: {e}" }
        },
        None => loading_grid(),
    };
    let gauges: Element = match snap.as_ref() {
        Some(Ok(o)) => gauges_row(o),
        _ => rsx! {},
    };
    drop(snap);

    let activity_snap = activity.read_unchecked();
    let charts: Element = match activity_snap.as_ref() {
        Some(Ok(rows))
            if rows.iter().any(|r| {
                r.sessions + r.attendance_joins + r.submissions + r.lessons_completed > 0
            }) =>
        {
            activity_section(rows)
        }
        // No activity yet / error / loading: the KPI grid carries the page.
        Some(Ok(_)) => rsx! {
            p { class: "muted", "Activity charts appear once your workspace has sessions, lessons, or submissions." }
        },
        Some(Err(_)) => rsx! {},
        None => rsx! { SkeletonCard { height: "220px".to_string() } },
    };
    drop(activity_snap);

    let body = rsx! {
        div { class: "admin-analytics-page",
            PageHeader {
                title: "Analytics".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: "Tenant-wide rollup of courses, members, sessions, and coursework.".to_string(),
            }
            { grid }
            { charts }
            { gauges }
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

    fn sample() -> OverviewDto {
        OverviewDto {
            courses_total: 12,
            courses_published: 7,
            members_students: 340,
            members_teachers: 18,
            members_tas: 5,
            members_parents: 22,
            sessions_last_30d: 64,
            sessions_ended_total: 410,
            recordings_available: 88,
            assignments_total: 130,
            submissions_total: 980,
            submissions_graded: 720,
        }
    }

    #[test]
    fn overview_grid_renders_metric_cards() {
        fn app_inner(o: OverviewDto) -> Element {
            overview_grid(&o)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-metric-card"), "got: {html}");
        // Headline values render.
        assert!(html.contains("Courses"));
        assert!(html.contains("7 published"));
        assert!(html.contains("Students"));
        assert!(html.contains("340"));
        assert!(html.contains("Sessions (30d)"));
        assert!(html.contains("720 graded"));
    }

    fn day(d: &str, att: i64, lessons: i64, subs: i64) -> DailyActivityDto {
        DailyActivityDto {
            day: d.into(),
            sessions: 1,
            attendance_joins: att,
            submissions: subs,
            lessons_completed: lessons,
        }
    }

    #[test]
    fn activity_section_renders_line_chart_with_data_expander() {
        fn app_inner(rows: Vec<DailyActivityDto>) -> Element {
            activity_section(&rows)
        }
        let rows = vec![
            day("2026-06-01", 5, 3, 2),
            day("2026-06-02", 8, 1, 0),
            day("2026-06-03", 2, 6, 4),
        ];
        let mut vdom = VirtualDom::new_with_props(app_inner, rows);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-chart--line"), "chart missing: {html}");
        assert!(html.contains("Engagement"), "label missing: {html}");
        assert!(
            html.contains("analytics-chart-clamp"),
            "chart height clamp wrapper missing: {html}"
        );
        // Accessible fallback table behind the expander.
        assert!(
            html.contains("chart-data-expander"),
            "expander missing: {html}"
        );
        assert!(html.contains("View data"), "summary missing: {html}");
        assert!(html.contains("ui-data-table"), "table missing: {html}");
        assert!(html.contains("2026-06-02"), "table data missing: {html}");
    }

    #[test]
    fn sparse_day_labels_keep_first_last_and_weekly() {
        let rows: Vec<DailyActivityDto> = (1..=10)
            .map(|i| day(&format!("2026-06-{i:02}"), 0, 0, 0))
            .collect();
        let labels = sparse_day_labels(&rows);
        assert_eq!(labels[0], "06-01");
        assert_eq!(labels[9], "06-10");
        assert_eq!(labels[7], "06-08"); // every 7th
        assert_eq!(labels[3], ""); // in-between days stay blank
    }

    #[test]
    fn gauges_row_renders_donuts_and_survives_zero_denominators() {
        fn app_inner(o: OverviewDto) -> Element {
            gauges_row(&o)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-donut-gauge"), "gauge missing: {html}");
        assert!(html.contains("Grading coverage"));
        assert!(html.contains("720 of 980 submissions graded"));

        // All-zero overview must not divide by zero.
        let zero = OverviewDto {
            courses_total: 0,
            courses_published: 0,
            members_students: 0,
            members_teachers: 0,
            members_tas: 0,
            members_parents: 0,
            sessions_last_30d: 0,
            sessions_ended_total: 0,
            recordings_available: 0,
            assignments_total: 0,
            submissions_total: 0,
            submissions_graded: 0,
        };
        let mut vdom = VirtualDom::new_with_props(app_inner, zero);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("0%"), "zero state should render 0%: {html}");
    }

    #[test]
    fn loading_grid_renders_skeletons() {
        fn app_inner() -> Element {
            loading_grid()
        }
        let mut vdom = VirtualDom::new(app_inner);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-skeleton-card"), "got: {html}");
    }
}
