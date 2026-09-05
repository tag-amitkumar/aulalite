// crates/features-courses/src/live_room_hand_raise.rs
//! Hand-raise UI. Student-side: raise/lower button. Teacher-side: queue list
//! with accept/dismiss controls.

use design_system::{Badge, BadgeTone, Button, ButtonVariant, EmptyState, EmptyStateVariant};
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct HandRaiseEntry {
    pub user_id: String,
    pub display_name: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomHandRaiseProps {
    pub is_teacher: bool,
    pub my_hand_raised: bool,
    pub queue: Vec<HandRaiseEntry>,
    pub on_raise: EventHandler<bool>,
    pub on_accept: EventHandler<String>,
    pub on_demote: EventHandler<String>,
}

pub fn LiveRoomHandRaise(props: LiveRoomHandRaiseProps) -> Element {
    rsx! {
        div { class: "live-room-hand-raise live-panel",
            if props.is_teacher {
                div { class: "live-panel-header",
                    h3 { class: "live-panel-title", "Hand-raise queue" }
                    if !props.queue.is_empty() {
                        Badge {
                            label: format!("{} pending", props.queue.len()),
                            tone: BadgeTone::Warning,
                        }
                    }
                }
                if props.queue.is_empty() {
                    EmptyState {
                        title: "No hands raised".to_string(),
                        description: "Students you accept will appear here.".to_string(),
                        variant: EmptyStateVariant::Subtle,
                        cta: None,
                    }
                } else {
                    ul { class: "hand-queue",
                        for entry in props.queue.iter() {
                            {
                                let id = entry.user_id.clone();
                                let id_for_demote = entry.user_id.clone();
                                let name = entry.display_name.clone();
                                let on_accept = props.on_accept;
                                let on_demote = props.on_demote;
                                rsx! {
                                    li { key: "{entry.user_id}", class: "hand-entry",
                                        span { class: "hand-name", "{name}" }
                                        Button {
                                            label: "Accept".to_string(),
                                            variant: ButtonVariant::Primary,
                                            on_click: move |_| on_accept.call(id.clone()),
                                        }
                                        Button {
                                            label: "Dismiss".to_string(),
                                            variant: ButtonVariant::Danger,
                                            on_click: move |_| on_demote.call(id_for_demote.clone()),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                {
                    let raised = props.my_hand_raised;
                    let on_raise = props.on_raise;
                    rsx! {
                        div { class: "hand-raise-cta",
                            if raised {
                                Badge {
                                    label: "Hand raised".to_string(),
                                    tone: BadgeTone::Warning,
                                }
                            }
                            Button {
                                label: if raised { "Lower hand".to_string() } else { "\u{270b} Raise hand".to_string() },
                                variant: if raised { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                                on_click: move |_| on_raise.call(!raised),
                            }
                        }
                    }
                }
            }
        }
    }
}
