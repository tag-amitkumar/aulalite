// crates/features-courses/src/course_people.rs
use design_system::{Badge, BadgeTone, Button, ButtonVariant, Card};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct Member {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub role: String,
    pub status: String,
}
#[derive(Clone, PartialEq)]
pub struct PendingInvite {
    pub id: String,
    pub email: String,
    pub role: String,
    pub expires_at: String,
}
#[derive(Clone, PartialEq)]
pub struct ActiveCode {
    pub id: String,
    pub last4: String,
    pub uses: i32,
    pub max_uses: Option<i32>,
}

#[derive(Props, Clone, PartialEq)]
pub struct CoursePeopleProps {
    pub members: Vec<Member>,
    pub pending_invites: Vec<PendingInvite>,
    pub active_codes: Vec<ActiveCode>,
    pub on_invite_clicked: EventHandler<()>,
    pub on_code_clicked: EventHandler<()>,
    pub on_revoke_invite: EventHandler<String>,
    pub on_revoke_code: EventHandler<String>,
}

#[component]
pub fn CoursePeople(props: CoursePeopleProps) -> Element {
    let on_invite_clicked = props.on_invite_clicked;
    let on_code_clicked = props.on_code_clicked;

    rsx! {
        div { class: "course-people",
            Card {
                div { class: "card-header",
                    h2 { "Members" }
                    div { class: "actions",
                        Button {
                            label: "Invite by email".to_string(),
                            variant: ButtonVariant::Primary,
                            on_click: move |_| on_invite_clicked.call(()),
                        }
                        Button {
                            label: "Generate code".to_string(),
                            variant: ButtonVariant::Secondary,
                            on_click: move |_| on_code_clicked.call(()),
                        }
                    }
                }
                table { class: "members-table",
                    thead { tr { th { "Name" } th { "Role" } th { "Status" } } }
                    tbody {
                        for m in &props.members {
                            tr {
                                td { "{m.display_name} ({m.email})" }
                                td { "{m.role}" }
                                td {
                                    Badge {
                                        label: m.status.clone(),
                                        tone: if m.status == "active" { BadgeTone::Success } else { BadgeTone::Neutral },
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !props.pending_invites.is_empty() {
                Card {
                    h2 { "Pending email invites" }
                    ul {
                        for inv in &props.pending_invites {
                            {
                                let id = inv.id.clone();
                                let revoke = props.on_revoke_invite;
                                rsx! {
                                    li {
                                        "{inv.email} ({inv.role}) — expires {inv.expires_at}"
                                        button {
                                            class: "linkish",
                                            onclick: move |_| revoke.call(id.clone()),
                                            "Revoke"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !props.active_codes.is_empty() {
                Card {
                    h2 { "Active enrollment codes" }
                    ul {
                        for code in &props.active_codes {
                            {
                                let id = code.id.clone();
                                let revoke = props.on_revoke_code;
                                let max_part = match code.max_uses {
                                    Some(m) => format!(" of {}", m),
                                    None => " (unlimited)".to_string(),
                                };
                                rsx! {
                                    li {
                                        "code …{code.last4} — {code.uses}{max_part} uses"
                                        button {
                                            class: "linkish",
                                            onclick: move |_| revoke.call(id.clone()),
                                            "Revoke"
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
