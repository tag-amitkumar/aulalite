// crates/features-courses/src/live_room_lobby.rs
use design_system::{Badge, BadgeTone, HeadingLevel, Loading, PageHeader, PageHeaderVariant};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomLobbyProps {
    pub course_title: String,
    pub instructor_name: Option<String>,
    pub scheduled_starts_at_iso: String,
}

pub fn LiveRoomLobby(props: LiveRoomLobbyProps) -> Element {
    let instructor = props
        .instructor_name
        .clone()
        .unwrap_or_else(|| "Your instructor".into());
    rsx! {
        div { class: "live-room-lobby motion-page",
            PageHeader {
                kicker: "Joining".to_string(),
                title: props.course_title.clone(),
                subtitle: "Class will begin shortly.".to_string(),
                variant: PageHeaderVariant::Hero,
                as_tag: HeadingLevel::H2,
            }
            div { class: "lobby-meta-row",
                Badge {
                    label: format!("Instructor · {instructor}"),
                    tone: BadgeTone::Neutral,
                }
                Badge {
                    label: format!("Starts {}", props.scheduled_starts_at_iso),
                    tone: BadgeTone::Info,
                }
            }
            div { class: "lobby-status-row",
                Loading {
                    message: "Waiting for the instructor to start the class\u{2026}".to_string(),
                }
            }
        }
    }
}
