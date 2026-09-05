// crates/features-courses/src/submissions_grading_table.rs
//! Teacher-only table listing all submissions for an assignment.

use crate::api::{self, ApiContext, AssignmentDto, SubmissionDto};
use design_system::{Badge, BadgeTone, Loading, Table};
use dioxus::prelude::*;

/// Shorten a user-id UUID to its first 8 characters for compact display.
/// The full id is surfaced via a `title=` tooltip at the call site.
fn short_user_id(id: &str) -> String {
    id.chars().take(8).collect()
}

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionsGradingTableProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
}

pub fn SubmissionsGradingTable(props: SubmissionsGradingTableProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment.id.clone();
    let submissions = use_resource(move || {
        let api = api.clone();
        let aid = aid.clone();
        async move { api::list_assignment_submissions(&api, &aid).await }
    });

    let mut selected: Signal<Option<SubmissionDto>> = use_signal(|| None);

    rsx! {
        div { class: "submissions-grading-table motion-page",
            h2 { "Submissions: {props.assignment.title}" }
            match &*submissions.read_unchecked() {
                Some(Ok(rows)) => {
                    let rows = rows.clone();
                    rsx! {
                        Table {
                            compact: true,
                            sticky_header: true,
                            head: rsx! {
                                tr {
                                    th { "Student" }
                                    th { "Status" }
                                    th { "Attempt" }
                                    th { "Submitted" }
                                    th { "Late?" }
                                    th { "Grade" }
                                    th { "Penalty" }
                                    th { "Released?" }
                                }
                            },
                            body: rsx! {
                                for s in rows.into_iter() {
                                    {
                                        let s_for_click = s.clone();
                                        let status_tone = match s.status.as_str() {
                                            "submitted" => BadgeTone::Info,
                                            "graded" => BadgeTone::Success,
                                            "returned" => BadgeTone::Warning,
                                            "draft" => BadgeTone::Neutral,
                                            _ => BadgeTone::Neutral,
                                        };
                                        // TODO: resolve display name once a names endpoint exists.
                                        // The submissions DTO only carries the raw student_user_id,
                                        // so show a shortened id with the full id as a tooltip.
                                        let full_id = s.student_user_id.clone();
                                        let short_id = short_user_id(&full_id);
                                        rsx! {
                                            tr { key: "{s.id}",
                                                onclick: move |_| selected.set(Some(s_for_click.clone())),
                                                td { title: "{full_id}", "{short_id}" }
                                                td { Badge { label: s.status.clone(), tone: status_tone } }
                                                td { "#{s.attempt_number}" }
                                                td { "{s.submitted_at.clone().unwrap_or_default()}" }
                                                td {
                                                    if s.is_late {
                                                        Badge { label: "LATE".to_string(), tone: BadgeTone::Warning }
                                                    }
                                                }
                                                td {
                                                    if let Some(n) = s.numeric_grade { "{n}" }
                                                    else if let Some(p) = s.passed {
                                                        if p { "PASS" } else { "FAIL" }
                                                    } else { "—" }
                                                }
                                                td {
                                                    {
                                                        match s.applied_late_penalty_percent {
                                                            Some(p) if p > 0 => rsx! {
                                                                Badge { label: format!("-{p}%"), tone: BadgeTone::Danger }
                                                            },
                                                            _ => rsx! { span { "—" } },
                                                        }
                                                    }
                                                }
                                                td {
                                                    if s.released_at.is_some() {
                                                        Badge { label: "released".to_string(), tone: BadgeTone::Success }
                                                    } else {
                                                        Badge { label: "held".to_string(), tone: BadgeTone::Neutral }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { Loading { message: "Loading submissions…".to_string() } },
            }
            if let Some(s) = selected.read().clone() {
                crate::submission_grade_modal::SubmissionGradeModal {
                    api: props.api.clone(),
                    assignment: props.assignment.clone(),
                    submission: s,
                    on_close: move |_| selected.set(None),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_user_id_truncates_to_eight_chars() {
        assert_eq!(
            short_user_id("123e4567-e89b-12d3-a456-426614174000"),
            "123e4567"
        );
        // Shorter than 8 → returned as-is.
        assert_eq!(short_user_id("abc"), "abc");
        assert_eq!(short_user_id(""), "");
    }
}
