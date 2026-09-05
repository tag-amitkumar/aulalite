// crates/features-courses/src/live_room_shell.rs
//! State-machine wrapper that picks Lobby / Broadcast / View based on
//! (caller_role, session_status).

use design_system::{EmptyState, EmptyStateVariant};
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub enum CallerRole {
    Teacher,
    Student,
}

#[derive(Clone, PartialEq, Debug)]
pub enum SessionStatus {
    Scheduled,
    Live,
    Ended,
    Cancelled,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Branch {
    Broadcast,
    Lobby,
    View,
    Ended,
    Cancelled,
}

pub fn route_for(role: &CallerRole, status: &SessionStatus) -> Branch {
    match (role, status) {
        (CallerRole::Teacher, SessionStatus::Scheduled) => Branch::Broadcast,
        (CallerRole::Teacher, SessionStatus::Live) => Branch::Broadcast,
        (CallerRole::Student, SessionStatus::Scheduled) => Branch::Lobby,
        (CallerRole::Student, SessionStatus::Live) => Branch::View,
        (_, SessionStatus::Ended) => Branch::Ended,
        (_, SessionStatus::Cancelled) => Branch::Cancelled,
    }
}

/// Whether this participant should keep re-asking the backend whether class
/// has started.
///
/// Defined AS "the lobby branch" rather than by repeating its condition, so it
/// can never drift from `route_for`: whoever is looking at the lobby is, by
/// definition, the person waiting for the class to begin.
///
/// Only students wait. A teacher drives the transition themselves by going
/// live, and re-polling underneath an active broadcast would churn props for
/// no reason. Polling stops the moment the session leaves `Scheduled` --
/// `Live`, `Ended` and `Cancelled` are all terminal for a lobby occupant.
pub fn should_poll_for_start(role: &CallerRole, status: &SessionStatus) -> bool {
    route_for(role, status) == Branch::Lobby
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomShellProps {
    pub session_id: String,
    pub course_slug: String,
    pub caller_role: CallerRole,
    pub status: SessionStatus,
    pub course_title: String,
    pub instructor_name: Option<String>,
    pub scheduled_starts_at_iso: String,
    pub transport_mode: String,
    pub viewer_jwt: Option<String>,
    pub main_url: Option<String>,
    pub screen_url: Option<String>,
    #[props(default)]
    pub has_recording: bool,
    #[props(default)]
    pub is_teacher: bool,
}

pub fn LiveRoomShell(props: LiveRoomShellProps) -> Element {
    let branch = route_for(&props.caller_role, &props.status);
    match branch {
        Branch::Broadcast => rsx! {
            div { class: "live-room-shell",
                crate::live_room_broadcast::LiveRoomBroadcast {
                    session_id: props.session_id.clone(),
                    status: props.status.clone(),
                }
            }
        },
        Branch::Lobby => rsx! {
            div { class: "live-room-shell",
                crate::live_room_lobby::LiveRoomLobby {
                    course_title: props.course_title.clone(),
                    instructor_name: props.instructor_name.clone(),
                    scheduled_starts_at_iso: props.scheduled_starts_at_iso.clone(),
                }
            }
        },
        Branch::View => rsx! {
            div { class: "live-room-shell",
                crate::live_room_view::LiveRoomView {
                    session_id: props.session_id.clone(),
                    transport_mode: props.transport_mode.clone(),
                    viewer_jwt: props.viewer_jwt.clone(),
                    main_url: props.main_url.clone(),
                    screen_url: props.screen_url.clone(),
                }
            }
        },
        Branch::Ended => {
            if props.has_recording {
                rsx! {
                    div { class: "live-room-shell",
                        crate::live_room_replay::LiveRoomReplay {
                            session_id: props.session_id.clone(),
                            course_title: props.course_title.clone(),
                            instructor_name: props.instructor_name.clone(),
                            is_teacher: props.is_teacher,
                        }
                    }
                }
            } else {
                rsx! {
                    div { class: "live-room-shell motion-page",
                        div { class: "live-room-ended",
                            EmptyState {
                                title: "Class has ended".to_string(),
                                description: "No recording is available for this session.".to_string(),
                                variant: EmptyStateVariant::Subtle,
                                cta: rsx! {
                                    a { class: "ds-button ds-button--secondary", href: "/", "Back to dashboard" }
                                },
                            }
                        }
                    }
                }
            }
        }
        Branch::Cancelled => rsx! {
            div { class: "live-room-shell motion-page",
                div { class: "live-room-cancelled",
                    EmptyState {
                        title: "Class was cancelled".to_string(),
                        description: "This session won't be held. Check the schedule for upcoming classes.".to_string(),
                        variant: EmptyStateVariant::Subtle,
                        cta: rsx! {
                            a { class: "ds-button ds-button--secondary", href: "/schedule", "View schedule" }
                        },
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teacher_scheduled_renders_broadcast() {
        assert_eq!(
            route_for(&CallerRole::Teacher, &SessionStatus::Scheduled),
            Branch::Broadcast
        );
    }
    #[test]
    fn teacher_live_renders_broadcast() {
        assert_eq!(
            route_for(&CallerRole::Teacher, &SessionStatus::Live),
            Branch::Broadcast
        );
    }
    #[test]
    fn student_scheduled_renders_lobby() {
        assert_eq!(
            route_for(&CallerRole::Student, &SessionStatus::Scheduled),
            Branch::Lobby
        );
    }
    #[test]
    fn student_live_renders_view() {
        assert_eq!(
            route_for(&CallerRole::Student, &SessionStatus::Live),
            Branch::View
        );
    }
    #[test]
    fn only_the_waiting_student_polls_for_class_start() {
        // The student staring at the lobby is exactly who needs to be told.
        assert!(should_poll_for_start(
            &CallerRole::Student,
            &SessionStatus::Scheduled
        ));
        // The teacher causes the transition; polling under them is pointless.
        assert!(!should_poll_for_start(
            &CallerRole::Teacher,
            &SessionStatus::Scheduled
        ));
    }

    #[test]
    fn polling_stops_once_the_session_leaves_scheduled() {
        for status in [
            SessionStatus::Live,
            SessionStatus::Ended,
            SessionStatus::Cancelled,
        ] {
            assert!(
                !should_poll_for_start(&CallerRole::Student, &status),
                "must stop polling at {status:?}"
            );
        }
    }

    #[test]
    fn polling_tracks_the_lobby_branch_exactly() {
        // The invariant that keeps the two in sync: poll iff branch == Lobby.
        for role in [CallerRole::Teacher, CallerRole::Student] {
            for status in [
                SessionStatus::Scheduled,
                SessionStatus::Live,
                SessionStatus::Ended,
                SessionStatus::Cancelled,
            ] {
                assert_eq!(
                    should_poll_for_start(&role, &status),
                    route_for(&role, &status) == Branch::Lobby,
                    "role={role:?} status={status:?}"
                );
            }
        }
    }

    #[test]
    fn ended_for_any_role() {
        assert_eq!(
            route_for(&CallerRole::Teacher, &SessionStatus::Ended),
            Branch::Ended
        );
        assert_eq!(
            route_for(&CallerRole::Student, &SessionStatus::Ended),
            Branch::Ended
        );
    }
    #[test]
    fn cancelled_for_any_role() {
        assert_eq!(
            route_for(&CallerRole::Teacher, &SessionStatus::Cancelled),
            Branch::Cancelled
        );
        assert_eq!(
            route_for(&CallerRole::Student, &SessionStatus::Cancelled),
            Branch::Cancelled
        );
    }
}
