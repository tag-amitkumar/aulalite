// crates/features-courses/src/analytics_panel.rs
//! Grade-analytics surfaces for the course Analytics tab: per-assignment grade
//! distribution, the class-average trend across recently-graded assignments,
//! and an at-risk student list.
//!
//! The tab itself is composed in `shell-web` (`routes/course_detail.rs`), which
//! already renders the metric cards, funnel, quiz distribution, and assignment
//! activity. This module adds three self-contained, pure (SSR-testable) render
//! helpers plus the DTO mirrors they consume, so the shell can drop them into
//! `analytics_body` without growing more inline chart code. They reuse the same
//! kinetics surfaces the rest of the tab uses — `LineChart` / `BarChart` for the
//! charts and `DataTable` for the at-risk list — so the look stays consistent.
//!
//! The DTOs mirror the backend additions in
//! `crates/backend/src/handlers/analytics.rs` (`GradeDistributionDto`,
//! `ClassAverageTrendDto`, `AtRiskStudentDto`). They're embedded in the
//! frontend `api::CourseAnalyticsDto` via new `#[serde(default)]` fields, so an
//! older backend that omits them simply yields empty vectors and the sections
//! render their empty states.

use design_system::kinetics_ui::{
    BarChart, ChartSeries, DataTable, DataTableColumn, DataTableRow, LineChart,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTO mirrors (decoded as part of api::CourseAnalyticsDto)
// ---------------------------------------------------------------------------

/// Mirrors the backend `GradeDistributionDto`: per-assignment percentage
/// buckets plus median/stddev over graded numeric submissions. `assignment_id`
/// is a Uuid serialized as a JSON string.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct GradeDistributionDto {
    pub assignment_id: String,
    pub title: String,
    pub graded_count: i64,
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
    pub median_pct: Option<f64>,
    pub stddev_pct: Option<f64>,
}

/// Mirrors the backend `ClassAverageTrendDto`: one graded numeric assignment
/// and the class mean percentage on it. Ordered oldest→newest by the backend.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ClassAverageTrendDto {
    pub assignment_id: String,
    pub title: String,
    pub graded_count: i64,
    pub avg_pct: f64,
}

/// Mirrors the backend `AtRiskStudentDto`: a student averaging below the
/// at-risk threshold across graded numeric work.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AtRiskStudentDto {
    pub student_user_id: String,
    pub display_name: String,
    pub avg_pct: f64,
    pub graded_count: i64,
}

// ---------------------------------------------------------------------------
// Pure formatting helpers
// ---------------------------------------------------------------------------

/// Format a 0.0–1.0(+) fraction as a whole-number percentage, e.g. 0.732 →
/// "73%". Clamps NaN/inf to 0 so a degenerate ratio never renders "NaN%".
fn pct(value: f64) -> String {
    if value.is_finite() {
        format!("{:.0}%", value * 100.0)
    } else {
        "0%".to_string()
    }
}

/// One-decimal percentage for the spread readout (median / stddev), e.g.
/// 0.084 → "8.4%". `None` renders as the em dash.
fn pct_opt(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{:.1}%", v * 100.0),
        _ => "—".to_string(),
    }
}

/// Truncate an assignment title for a chart axis label (mirrors the shell's
/// `axis_label` so the trend line and the assignment-activity chart agree).
fn axis_label(title: &str) -> String {
    if title.chars().count() > 12 {
        let cut: String = title.chars().take(11).collect();
        format!("{cut}…")
    } else {
        title.to_string()
    }
}

// ---------------------------------------------------------------------------
// Render helpers (pure; SSR-testable)
// ---------------------------------------------------------------------------

/// Class-average trend across the last N graded numeric assignments, plotted as
/// a percentage line (0–100%). Renders an empty-state note when there is no
/// graded numeric work yet. The accessible data fallback lives in a "View data"
/// expander, matching the assignment-activity chart in the shell.
pub fn analytics_grade_trend(rows: &[ClassAverageTrendDto]) -> Element {
    if rows.is_empty() {
        return rsx! {
            section { class: "analytics-grade-trend",
                h3 { class: "analytics-section-title", "Class average trend" }
                p { class: "muted",
                    "No graded numeric assignments yet — the trend appears once work is graded." }
            }
        };
    }
    // Plot as percentages so the axis reads 0–100 regardless of max_points.
    let series = vec![ChartSeries::new(
        "Class average %",
        rows.iter().map(|r| (r.avg_pct * 100.0) as f32).collect(),
    )];
    let x_labels: Vec<String> = rows.iter().map(|r| axis_label(&r.title)).collect();

    let columns = vec![
        DataTableColumn::new("assignment", "Assignment"),
        DataTableColumn::new("graded", "Graded"),
        DataTableColumn::new("avg", "Class avg"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|r| {
            DataTableRow::new(
                r.assignment_id.clone(),
                vec![r.title.clone(), r.graded_count.to_string(), pct(r.avg_pct)],
            )
        })
        .collect();

    rsx! {
        section { class: "analytics-grade-trend",
            h3 { class: "analytics-section-title", "Class average trend" }
            LineChart {
                label: "Class average across recent graded assignments".to_string(),
                series,
                x_labels,
                show_legend: false,
                show_area: true,
            }
            details { class: "chart-data-expander",
                summary { "View data" }
                DataTable {
                    columns,
                    rows: table_rows,
                    caption: "Class average per assignment".to_string(),
                }
            }
        }
    }
}

/// Per-assignment grade distribution: a grouped bar chart of the four
/// percentage bands per graded numeric assignment, with median/stddev exposed
/// in the "View data" table. Renders an empty-state note when nothing is graded.
pub fn analytics_grade_distribution(rows: &[GradeDistributionDto]) -> Element {
    if rows.is_empty() {
        return rsx! {
            section { class: "analytics-grade-distribution",
                h3 { class: "analytics-section-title", "Grade distribution" }
                p { class: "muted",
                    "No graded numeric assignments yet — distribution appears once work is graded." }
            }
        };
    }
    let series = vec![
        ChartSeries::new("<50%", rows.iter().map(|r| r.under_50 as f32).collect()),
        ChartSeries::new(
            "50–69%",
            rows.iter().map(|r| r.from_50_to_69 as f32).collect(),
        ),
        ChartSeries::new(
            "70–89%",
            rows.iter().map(|r| r.from_70_to_89 as f32).collect(),
        ),
        ChartSeries::new("90%+", rows.iter().map(|r| r.from_90_up as f32).collect()),
    ];
    let x_labels: Vec<String> = rows.iter().map(|r| axis_label(&r.title)).collect();

    let columns = vec![
        DataTableColumn::new("assignment", "Assignment"),
        DataTableColumn::new("graded", "Graded"),
        DataTableColumn::new("under50", "<50%"),
        DataTableColumn::new("b50", "50–69%"),
        DataTableColumn::new("b70", "70–89%"),
        DataTableColumn::new("b90", "90%+"),
        DataTableColumn::new("median", "Median"),
        DataTableColumn::new("stddev", "Std dev"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|r| {
            DataTableRow::new(
                r.assignment_id.clone(),
                vec![
                    r.title.clone(),
                    r.graded_count.to_string(),
                    r.under_50.to_string(),
                    r.from_50_to_69.to_string(),
                    r.from_70_to_89.to_string(),
                    r.from_90_up.to_string(),
                    pct_opt(r.median_pct),
                    pct_opt(r.stddev_pct),
                ],
            )
        })
        .collect();

    rsx! {
        section { class: "analytics-grade-distribution",
            h3 { class: "analytics-section-title", "Grade distribution" }
            BarChart {
                label: "Grade distribution by assignment".to_string(),
                series,
                x_labels,
            }
            details { class: "chart-data-expander",
                summary { "View data" }
                DataTable {
                    columns,
                    rows: table_rows,
                    caption: "Grade buckets, median, and spread per assignment".to_string(),
                }
            }
        }
    }
}

/// At-risk student list: students whose average across graded numeric work is
/// below `threshold` (0.0–1.0). Renders a reassuring note when the list is empty
/// so staff can tell "nobody at risk" apart from "no data". The threshold is
/// shown in the caption so the bar is explicit.
pub fn analytics_at_risk(rows: &[AtRiskStudentDto], threshold: f64) -> Element {
    let threshold_label = pct(threshold);
    if rows.is_empty() {
        return rsx! {
            section { class: "analytics-at-risk",
                h3 { class: "analytics-section-title", "At-risk students" }
                p { class: "muted",
                    "No students are averaging below {threshold_label} across graded work." }
            }
        };
    }
    let columns = vec![
        DataTableColumn::new("student", "Student"),
        DataTableColumn::new("avg", "Average"),
        DataTableColumn::new("graded", "Graded items"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|r| {
            DataTableRow::new(
                r.student_user_id.clone(),
                vec![
                    r.display_name.clone(),
                    pct(r.avg_pct),
                    r.graded_count.to_string(),
                ],
            )
        })
        .collect();

    rsx! {
        section { class: "analytics-at-risk",
            h3 { class: "analytics-section-title", "At-risk students" }
            p { class: "muted",
                "Students averaging below {threshold_label} across graded numeric work." }
            DataTable {
                columns,
                rows: table_rows,
                caption: format!("Students below {threshold_label} average"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trend_sample() -> Vec<ClassAverageTrendDto> {
        vec![
            ClassAverageTrendDto {
                assignment_id: "a1".to_string(),
                title: "Essay 1".to_string(),
                graded_count: 20,
                avg_pct: 0.82,
            },
            ClassAverageTrendDto {
                assignment_id: "a2".to_string(),
                title: "Midterm".to_string(),
                graded_count: 18,
                avg_pct: 0.64,
            },
        ]
    }

    #[test]
    fn pct_formats_and_clamps() {
        assert_eq!(pct(0.732), "73%");
        assert_eq!(pct(0.0), "0%");
        assert_eq!(pct(f64::NAN), "0%");
        assert_eq!(pct_opt(Some(0.084)), "8.4%");
        assert_eq!(pct_opt(None), "—");
    }

    #[test]
    fn grade_trend_renders_chart_and_data() {
        fn app(rows: Vec<ClassAverageTrendDto>) -> Element {
            analytics_grade_trend(&rows)
        }
        let mut vdom = VirtualDom::new_with_props(app, trend_sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-chart"), "expected a chart: {html}");
        assert!(html.contains("Class average trend"), "got: {html}");
        assert!(html.contains("Midterm"), "got: {html}");
    }

    #[test]
    fn grade_trend_empty_state() {
        fn app(rows: Vec<ClassAverageTrendDto>) -> Element {
            analytics_grade_trend(&rows)
        }
        let mut vdom = VirtualDom::new_with_props(app, Vec::<ClassAverageTrendDto>::new());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("No graded numeric assignments yet"),
            "got: {html}"
        );
    }

    #[test]
    fn grade_distribution_renders_buckets_and_spread() {
        fn app(rows: Vec<GradeDistributionDto>) -> Element {
            analytics_grade_distribution(&rows)
        }
        let rows = vec![GradeDistributionDto {
            assignment_id: "a1".to_string(),
            title: "Quiz 1".to_string(),
            graded_count: 10,
            under_50: 1,
            from_50_to_69: 2,
            from_70_to_89: 4,
            from_90_up: 3,
            median_pct: Some(0.78),
            stddev_pct: Some(0.12),
        }];
        let mut vdom = VirtualDom::new_with_props(app, rows);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-chart"), "expected a chart: {html}");
        assert!(html.contains("Grade distribution"), "got: {html}");
        assert!(
            html.contains("ui-data-table"),
            "expected a data table: {html}"
        );
        // Median rendered as a one-decimal percentage.
        assert!(html.contains("78.0%"), "median missing: {html}");
    }

    #[test]
    fn at_risk_renders_list_and_empty_note() {
        fn app(rows: Vec<AtRiskStudentDto>) -> Element {
            analytics_at_risk(&rows, 0.60)
        }
        let rows = vec![AtRiskStudentDto {
            student_user_id: "u1".to_string(),
            display_name: "Avery".to_string(),
            avg_pct: 0.42,
            graded_count: 5,
        }];
        let mut vdom = VirtualDom::new_with_props(app, rows);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ui-data-table"),
            "expected a data table: {html}"
        );
        assert!(html.contains("Avery"), "got: {html}");
        assert!(html.contains("42%"), "avg pct missing: {html}");

        // Empty case shows the reassuring note with the threshold.
        let mut vdom2 = VirtualDom::new_with_props(app, Vec::<AtRiskStudentDto>::new());
        vdom2.rebuild_in_place();
        let html2 = dioxus_ssr::render(&vdom2);
        assert!(
            html2.contains("No students are averaging below 60%"),
            "got: {html2}"
        );
    }
}
