// crates/features-courses/src/live_room_presence.rs
//! Presence indicator. Asymmetric: teachers see full list; students see count.

use design_system::{Avatar, AvatarSize, Badge, BadgeTone};
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct PresenceParticipant {
    pub user_id: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomPresenceProps {
    pub count: u32,
    pub participants: Option<Vec<PresenceParticipant>>,
    pub is_teacher: bool,
}

pub fn LiveRoomPresence(props: LiveRoomPresenceProps) -> Element {
    rsx! {
        div { class: "live-room-presence live-panel",
            div { class: "presence-count",
                Badge {
                    label: format!("{} watching", props.count),
                    tone: BadgeTone::Live,
                }
            }
            if props.is_teacher {
                if let Some(list) = &props.participants {
                    div { class: "presence-list",
                        h4 { class: "live-panel-title", "Participants" }
                        ul { class: "participant-list",
                            for p in list.iter() {
                                {
                                    let role_tone = if p.role.eq_ignore_ascii_case("teacher") {
                                        BadgeTone::Success
                                    } else {
                                        BadgeTone::Neutral
                                    };
                                    rsx! {
                                        li { key: "{p.user_id}", class: "participant",
                                            Avatar {
                                                name: p.display_name.clone(),
                                                size: AvatarSize::Sm,
                                            }
                                            span { class: "participant-name", "{p.display_name}" }
                                            Badge {
                                                label: p.role.clone(),
                                                tone: role_tone,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
