// crates/features-courses/src/assignment_list.rs
//! Course-level list of assignments. Teachers see drafts + published;
//! students see published only.

use crate::api::{self, ApiContext, AssignmentDto};
use design_system::{Badge, BadgeTone, Card, CardList, EmptyState, SkeletonLine};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentListProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub course_id: String,
    /// Teachers/admins may create coursework; TAs cannot.
    pub can_author: bool,
    /// Assigned teachers, TAs, and admins may see drafts.
    pub can_view_drafts: bool,
}

pub fn AssignmentList(props: AssignmentListProps) -> Element {
    let course_id = props.course_id.clone();
    let api = props.api.clone();
    let can_view_drafts = props.can_view_drafts;

    let assignments = use_resource(move || {
        let api = api.clone();
        let course_id = course_id.clone();
        async move { api::list_course_assignments(&api, &course_id, can_view_drafts).await }
    });

    let new_href = format!("/courses/{}/assignments/new", props.course_slug);
    let new_href_for_empty_cta = new_href.clone();

    rsx! {
        div { class: "assignment-list assignment-shell motion-page",
            div { class: "assignment-list__header",
                h2 { "Assignments" }
                if props.can_author {
                    a {
                        href: "{new_href}",
                        class: "ds-button ds-button--primary",
                        "New assignment"
                    }
                }
            }
            match &*assignments.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "No assignments yet".to_string(),
                        description: "Published work, drafts, and grading activity will appear here.".to_string(),
                        cta: if props.can_author {
                            Some(rsx! {
                                a {
                                    href: "{new_href_for_empty_cta}",
                                    class: "ds-button ds-button--primary",
                                    "New assignment"
                                }
                            })
                        } else {
                            None
                        },
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for a in items.iter() {
                            { render_row(props.course_slug.clone(), a) }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "{e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "70%".to_string() }
                        SkeletonLine { width: "60%".to_string() }
                    }
                },
            }
        }
    }
}

fn render_row(course_slug: String, a: &AssignmentDto) -> Element {
    let id = a.id.clone();
    let title = a.title.clone();
    let status = a.status.clone();
    let due = a.due_at.clone().unwrap_or_default();
    let tone = match status.as_str() {
        "draft" => BadgeTone::Neutral,
        "published" => BadgeTone::Success,
        "archived" => BadgeTone::Warning,
        _ => BadgeTone::Neutral,
    };
    rsx! {
        li { key: "{id}", class: "assignment-list__row",
            Card {
                div { class: "row-1",
                    a { href: format!("/courses/{course_slug}/assignments/{id}"),
                        class: "assignment-list__title",
                        "{title}"
                    }
                    Badge { label: status, tone }
                }
                if !due.is_empty() {
                    div { class: "row-2",
                        span { class: "due", "due {due}" }
                    }
                }
            }
        }
    }
}
