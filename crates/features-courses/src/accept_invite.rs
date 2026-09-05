// crates/features-courses/src/accept_invite.rs
use design_system::{Card, EmptyState, EmptyStateVariant, Loading};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub enum AcceptState {
    Loading,
    Success {
        course_title: String,
        course_slug: String,
    },
    Failure {
        reason: String,
    },
}

#[derive(Props, Clone, PartialEq)]
pub struct AcceptInviteProps {
    pub state: AcceptState,
}

#[component]
pub fn AcceptInvite(props: AcceptInviteProps) -> Element {
    rsx! {
        div { class: "accept-page workflow-page motion-page",
            Card {
                match &props.state {
                    AcceptState::Loading => rsx! {
                        Loading { message: "Accepting your invitation…".to_string() }
                    },
                    AcceptState::Success { course_title, course_slug } => rsx! {
                        EmptyState {
                            variant: EmptyStateVariant::Accent,
                            title: format!("Welcome to {course_title}"),
                            description: "You're now enrolled.".to_string(),
                            cta: rsx! {
                                a { class: "ds-button ds-button--primary", href: "/courses/{course_slug}", "Go to course →" }
                            },
                        }
                    },
                    AcceptState::Failure { reason } => rsx! {
                        EmptyState {
                            variant: EmptyStateVariant::Default,
                            title: "We couldn't accept this invite".to_string(),
                            description: reason.clone(),
                            cta: rsx! {
                                a { class: "ds-button ds-button--secondary", href: "/", "Back to dashboard" }
                            },
                        }
                    },
                }
            }
        }
    }
}
