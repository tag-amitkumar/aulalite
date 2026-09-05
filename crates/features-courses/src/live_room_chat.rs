// crates/features-courses/src/live_room_chat.rs
//! Chat sidebar component for the live room.

use design_system::{Button, ButtonVariant, EmptyState, EmptyStateVariant, Field, Input};
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub sender_display_name: String,
    pub body: String,
    pub created_at: String,
    pub deleted: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomChatProps {
    pub messages: Vec<ChatMessage>,
    pub is_teacher: bool,
    pub on_send: EventHandler<String>,
    pub on_delete: EventHandler<String>,
}

pub fn LiveRoomChat(props: LiveRoomChatProps) -> Element {
    let mut input = use_signal(String::new);

    let on_send = props.on_send;
    let send = move |_| {
        let text = input.read().clone();
        if !text.trim().is_empty() {
            on_send.call(text);
            input.set(String::new());
        }
    };

    rsx! {
        div { class: "live-room-chat live-panel",
            h3 { class: "live-panel-title", "Chat" }
            div { class: "chat-messages",
                if props.messages.is_empty() {
                    EmptyState {
                        title: "No messages yet".to_string(),
                        description: "Be the first to say hello.".to_string(),
                        variant: EmptyStateVariant::Subtle,
                        cta: None,
                    }
                } else {
                    ul { class: "chat-list",
                        for msg in props.messages.iter() {
                            {
                                let id = msg.id.clone();
                                let sender = msg.sender_display_name.clone();
                                let body = msg.body.clone();
                                let deleted = msg.deleted;
                                let on_delete = props.on_delete;
                                let is_teacher = props.is_teacher;
                                rsx! {
                                    li { key: "{id}", class: if deleted { "chat-msg deleted" } else { "chat-msg" },
                                        span { class: "chat-sender", "{sender}: " }
                                        if deleted {
                                            span { class: "chat-body deleted", "[deleted]" }
                                        } else {
                                            span { class: "chat-body", "{body}" }
                                        }
                                        if is_teacher && !deleted {
                                            Button {
                                                label: "\u{00d7}".to_string(),
                                                variant: ButtonVariant::Ghost,
                                                on_click: move |_| on_delete.call(id.clone()),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "chat-input",
                Field {
                    label: "Send a message".to_string(),
                    Input {
                        value: input.read().clone(),
                        placeholder: "Type a message\u{2026}".to_string(),
                        on_input: move |val: String| input.set(val),
                    }
                }
                Button {
                    label: "Send".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: send,
                }
            }
        }
    }
}
