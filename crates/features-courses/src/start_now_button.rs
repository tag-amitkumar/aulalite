// crates/features-courses/src/start_now_button.rs
//! Split button rendered in the CourseDetail header for admins. Primary
//! click starts an ad-hoc session with defaults; caret opens
//! `StartNowModal` for customization. When a session is already live in
//! the course (driven by `use_active_session_poll`), the button relabels
//! to "Join active session" and the caret hides.

use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum StartNowState {
    /// No live session — show "Start session now" + caret.
    Idle,
    /// A live session exists — show "Join active session" only.
    Conflict { active_session_id: String },
    /// Posting `start-now`. Button disabled with spinner-style label.
    Submitting,
}

#[derive(Props, Clone, PartialEq)]
pub struct StartNowButtonProps {
    pub state: StartNowState,
    pub on_quick_start: EventHandler<()>,
    pub on_customize: EventHandler<()>,
    pub on_join_active: EventHandler<String>,
}

#[component]
pub fn StartNowButton(props: StartNowButtonProps) -> Element {
    match &props.state {
        StartNowState::Idle => rsx! {
            span { class: "start-now-split",
                Button {
                    label: "Start session now".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_quick_start.call(()),
                }
                Button {
                    label: "▾".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_customize.call(()),
                }
            }
        },
        StartNowState::Submitting => rsx! {
            Button {
                label: "Starting…".to_string(),
                variant: ButtonVariant::Primary,
                button_type: "button".to_string(),
                disabled: true,
                on_click: move |_| {},
            }
        },
        StartNowState::Conflict { active_session_id } => {
            let id = active_session_id.clone();
            rsx! {
                Button {
                    label: "Join active session".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_join_active.call(id.clone()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_state_renders_primary_plus_caret() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Idle,
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Start session now"),
            "missing primary: {html}"
        );
        assert!(html.contains("▾"), "missing caret: {html}");
    }

    #[test]
    fn conflict_state_relabels_and_hides_caret() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Conflict { active_session_id: "session-1".to_string() },
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Join active session"),
            "missing relabeled button: {html}"
        );
        assert!(!html.contains("▾"), "caret should be hidden: {html}");
        assert!(
            !html.contains("Start session now"),
            "primary label should be hidden: {html}"
        );
    }

    #[test]
    fn submitting_state_disables_button() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Submitting,
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Starting…"),
            "missing submitting label: {html}"
        );
        assert!(
            html.contains("disabled"),
            "button should be disabled: {html}"
        );
    }
}
