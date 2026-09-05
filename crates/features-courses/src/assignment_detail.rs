// crates/features-courses/src/assignment_detail.rs
//! Single-assignment view. Students see their submission card; teachers see
//! the link to the grading table.

use crate::api::{self, ApiContext};
use design_system::{Badge, BadgeTone, PageHeader, SkeletonLine};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentDetailProps {
    pub api: ApiContext,
    pub assignment_id: String,
    pub course_slug: String,
    pub current_user_id: String,
    pub can_author: bool,
    pub can_grade: bool,
}

/// Build a short, single-line subtitle from the assignment's markdown
/// instructions. Strips newlines/extra whitespace and truncates to ~120 chars.
fn subtitle_from_instructions(md: &str) -> Option<String> {
    let trimmed: String = md.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        None
    } else if trimmed.chars().count() > 120 {
        let head: String = trimmed.chars().take(117).collect();
        Some(format!("{head}…"))
    } else {
        Some(trimmed)
    }
}

pub fn AssignmentDetail(props: AssignmentDetailProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment_id.clone();
    let assignment = use_resource(move || {
        let api = api.clone();
        let aid = aid.clone();
        async move { api::get_assignment(&api, &aid).await }
    });

    rsx! {
        div { class: "assignment-detail assignment-shell motion-page",
            match &*assignment.read_unchecked() {
                Some(Ok(a)) => {
                    let subtitle = subtitle_from_instructions(&a.instructions_md);
                    let grade_href = format!(
                        "/courses/{}/assignments/{}/grade",
                        props.course_slug, props.assignment_id,
                    );
                    let header_actions: Option<Element> = if a.status == "draft"
                        && (props.can_author || props.can_grade)
                    {
                        Some(rsx! { Badge { label: "draft".to_string(), tone: BadgeTone::Neutral } })
                    } else {
                        None
                    };
                    rsx! {
                        PageHeader {
                            kicker: "Assignment".to_string(),
                            title: a.title.clone(),
                            subtitle: subtitle,
                            actions: header_actions,
                        }
                        if let Some(due) = a.due_at.as_ref() {
                            p { class: "due", "Due: {due}" }
                        }
                        div { class: "assignment-detail__instructions",
                            p { class: "instructions", "{a.instructions_md}" }
                        }
                        if props.can_grade {
                            div { class: "assignment-detail__actions",
                                a {
                                    href: "{grade_href}",
                                    class: "ds-button ds-button--primary",
                                    "View submissions"
                                }
                            }
                        } else {
                            crate::submission_form::SubmissionForm {
                                api: props.api.clone(),
                                assignment: a.clone(),
                            }
                        }
                        crate::peer_review::PeerReview {
                            assignment_id: props.assignment_id.clone(),
                            is_teacher: props.can_grade,
                        }
                    }
                },
                Some(Err(e)) => rsx! { div { class: "system-state system-state--error", "{e}" } },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "60%".to_string(), height: "32px".to_string() }
                        SkeletonLine { width: "85%".to_string() }
                        SkeletonLine { width: "70%".to_string() }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_truncates_long_instructions() {
        let md = "a ".repeat(200);
        let s = subtitle_from_instructions(&md).unwrap();
        assert!(s.ends_with('…'), "expected ellipsis: {s}");
        assert!(s.chars().count() <= 120, "too long: {s}");
    }

    #[test]
    fn subtitle_collapses_whitespace() {
        let md = "Read\n\n\nchapter   one.";
        let s = subtitle_from_instructions(md).unwrap();
        assert_eq!(s, "Read chapter one.");
    }

    #[test]
    fn subtitle_empty_returns_none() {
        assert!(subtitle_from_instructions("").is_none());
        assert!(subtitle_from_instructions("   \n  \t").is_none());
    }
}
