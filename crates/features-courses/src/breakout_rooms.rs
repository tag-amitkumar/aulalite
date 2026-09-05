// crates/features-courses/src/breakout_rooms.rs
//! Live breakout rooms for the live room — a teacher panel (create rooms +
//! auto-split + assign participants + open/close) and a student banner that
//! auto-rejoins the assigned sub-room.
//!
//! Breakouts are ephemeral, driven entirely over the live-room socket (mirrors
//! polls/reactions): the teacher sends `create_breakouts` / `assign_breakout` /
//! `open_breakouts` / `close_breakouts`, and the server fans `breakout_opened` /
//! `breakout_updated` / `breakout_closed` back to the room plus a targeted
//! `breakout_assignment` to each assigned participant. This module owns the
//! client-side breakout state machine (`BreakoutClientState`) plus the two
//! components; the views translate socket events into `apply_*` calls and
//! forward the button callbacks onto the socket.

use design_system::SelectOption;
use dioxus::prelude::*;

/// Max breakout rooms, mirroring `backend::services::live_room::BREAKOUT_MAX_ROOMS`.
pub const BREAKOUT_MAX_ROOMS: usize = 20;

/// One breakout room as the client knows it. `id` is the room UUID string used
/// for assignment commands; `members` are participant user-id strings.
#[derive(Clone, PartialEq, Debug)]
pub struct BreakoutRoomView {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

/// Breakout state as the client knows it. `open` flips the student banner and
/// the teacher Open/Close control.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct BreakoutClientState {
    pub open: bool,
    pub rooms: Vec<BreakoutRoomView>,
    /// This client's own assignment (room id + name), set from the targeted
    /// `breakout_assignment` event. `None` means the main room.
    pub my_room: Option<MyAssignment>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MyAssignment {
    pub room_id: String,
    pub room_name: String,
    /// Full WHEP URL to (re)subscribe to while assigned.
    pub whep_url: String,
}

impl BreakoutClientState {
    /// `breakout_opened`: the layout is now open with these rooms.
    pub fn on_opened(&mut self, rooms: Vec<BreakoutRoomView>) {
        self.open = true;
        self.rooms = rooms;
    }

    /// `breakout_updated`: the layout changed while open.
    pub fn on_updated(&mut self, rooms: Vec<BreakoutRoomView>) {
        self.rooms = rooms;
    }

    /// `breakout_closed`: everyone returns to the main room.
    pub fn on_closed(&mut self) {
        self.open = false;
        self.rooms.clear();
        self.my_room = None;
    }

    /// Hydrate from a `breakout_snapshot` sent on (re)connect.
    pub fn on_snapshot(&mut self, open: bool, rooms: Vec<BreakoutRoomView>) {
        self.open = open;
        self.rooms = if open { rooms } else { Vec::new() };
        if !open {
            self.my_room = None;
        }
    }

    /// `breakout_assignment` targeted at this client. `room_id == None` means
    /// the main room. Returns the WHEP URL to (re)subscribe to, or `None` for
    /// the main room.
    pub fn on_my_assignment(
        &mut self,
        room_id: Option<String>,
        room_name: String,
        whep_url: String,
    ) -> Option<String> {
        match room_id {
            Some(id) => {
                let url = whep_url.clone();
                self.my_room = Some(MyAssignment {
                    room_id: id,
                    room_name,
                    whep_url,
                });
                Some(url)
            }
            None => {
                self.my_room = None;
                None
            }
        }
    }

    /// Count of participants currently placed in any breakout room.
    pub fn assigned_count(&self) -> usize {
        self.rooms.iter().map(|r| r.members.len()).sum()
    }
}

// ===========================================================================
// Student banner — rendered in the student view while breakouts are open.
// ===========================================================================

#[derive(Props, Clone, PartialEq)]
pub struct BreakoutBannerProps {
    pub state: BreakoutClientState,
}

/// The participant-facing banner. While breakouts are open it shows whether the
/// student is in a named breakout room (auto-rejoined to its feed) or waiting
/// in the main room for an assignment.
#[allow(non_snake_case)]
pub fn BreakoutBanner(props: BreakoutBannerProps) -> Element {
    if !props.state.open {
        return rsx! {};
    }
    let in_room = props.state.my_room.clone();
    rsx! {
        section { class: "breakout-banner", "aria-label": "Breakout rooms",
            match in_room {
                Some(assignment) => rsx! {
                    div { class: "breakout-banner-row breakout-banner-row--assigned",
                        span { class: "breakout-banner-eyebrow", "Breakout room" }
                        span { class: "breakout-banner-room", "{assignment.room_name}" }
                        span { class: "breakout-banner-hint",
                            "You've been moved to a breakout room. Audio and video follow automatically."
                        }
                    }
                },
                None => rsx! {
                    div { class: "breakout-banner-row breakout-banner-row--waiting",
                        span { class: "breakout-banner-eyebrow", "Breakout rooms are open" }
                        span { class: "breakout-banner-hint",
                            "Waiting for the teacher to assign you to a room."
                        }
                    }
                },
            }
        }
    }
}

// ===========================================================================
// Teacher panel — rendered only in the broadcast view.
// ===========================================================================

#[derive(Props, Clone, PartialEq)]
pub struct BreakoutPanelProps {
    pub state: BreakoutClientState,
    /// Current room participants (user_id, display_name) the teacher can assign.
    pub participants: Vec<(String, String)>,
    /// Fires with the desired room count for an auto-split.
    pub on_auto_split: EventHandler<usize>,
    /// Fires with the list of room names to create (manual setup).
    pub on_create: EventHandler<Vec<String>>,
    /// Fires with `(user_id, room_id_or_empty)` — empty `room_id` returns the
    /// user to the main room.
    pub on_assign: EventHandler<(String, String)>,
    /// Fires when the teacher opens the created rooms.
    pub on_open: EventHandler<()>,
    /// Fires when the teacher closes all breakouts.
    pub on_close: EventHandler<()>,
}

/// Teacher-only breakout panel. When no rooms exist it offers an auto-split
/// control; once rooms exist it lists them with per-participant assignment and
/// an Open/Close control.
#[allow(non_snake_case)]
pub fn BreakoutPanel(props: BreakoutPanelProps) -> Element {
    let mut room_count = use_signal(|| 2usize);

    let has_rooms = !props.state.rooms.is_empty();
    let is_open = props.state.open;
    let participants = props.participants.clone();

    rsx! {
        section { class: "breakout-panel", "aria-label": "Breakout rooms",
            div { class: "breakout-panel-header",
                span { class: "breakout-panel-eyebrow", "Breakout rooms" }
                if is_open {
                    span { class: "breakout-panel-badge breakout-panel-badge--open", "Open" }
                }
            }

            if !has_rooms {
                div { class: "breakout-setup",
                    label { class: "breakout-field-label", "Number of rooms" }
                    div { class: "breakout-setup-row",
                        design_system::Select {
                            value: room_count.read().to_string(),
                            options: (1..=8).map(|n| SelectOption {
                                value: n.to_string(),
                                label: format!("{n} rooms"),
                            }).collect::<Vec<_>>(),
                            on_change: move |v: String| {
                                if let Ok(n) = v.parse::<usize>() { room_count.set(n); }
                            },
                        }
                        design_system::Button {
                            label: "Auto-split".to_string(),
                            variant: design_system::ButtonVariant::Primary,
                            size: design_system::ButtonSize::Sm,
                            on_click: move |_| props.on_auto_split.call(*room_count.read()),
                        }
                    }
                    p { class: "breakout-setup-hint",
                        "Auto-split distributes everyone currently in the room evenly across the rooms. You can fine-tune assignments after."
                    }
                }
            } else {
                div { class: "breakout-rooms-list",
                    for room in props.state.rooms.iter() {
                        {
                            let room = room.clone();
                            let member_count = room.members.len();
                            rsx! {
                                div { key: "{room.id}", class: "breakout-room-card",
                                    div { class: "breakout-room-head",
                                        span { class: "breakout-room-name", "{room.name}" }
                                        span { class: "breakout-room-count", "{member_count}" }
                                    }
                                    ul { class: "breakout-room-members", role: "list",
                                        for member_id in room.members.iter() {
                                            {
                                                let label = participants
                                                    .iter()
                                                    .find(|(uid, _)| uid == member_id)
                                                    .map(|(_, name)| name.clone())
                                                    .unwrap_or_else(|| {
                                                        let short: String = member_id.chars().take(8).collect();
                                                        format!("user-{short}")
                                                    });
                                                rsx! {
                                                    li { key: "{member_id}", class: "breakout-room-member", "{label}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Per-participant assignment picker.
                div { class: "breakout-assign-list",
                    span { class: "breakout-field-label", "Assign participants" }
                    for (uid, name) in participants.iter() {
                        {
                            let uid = uid.clone();
                            let name = name.clone();
                            let current_room = props.state.rooms
                                .iter()
                                .find(|r| r.members.iter().any(|m| m == &uid))
                                .map(|r| r.id.clone())
                                .unwrap_or_default();
                            let mut opts: Vec<SelectOption> = vec![SelectOption {
                                value: String::new(),
                                label: "Main room".to_string(),
                            }];
                            opts.extend(props.state.rooms.iter().map(|r| SelectOption {
                                value: r.id.clone(),
                                label: r.name.clone(),
                            }));
                            rsx! {
                                div { key: "{uid}", class: "breakout-assign-row",
                                    span { class: "breakout-assign-name", "{name}" }
                                    design_system::Select {
                                        value: current_room,
                                        options: opts,
                                        on_change: move |room_id: String| {
                                            props.on_assign.call((uid.clone(), room_id));
                                        },
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "breakout-panel-actions",
                    if is_open {
                        design_system::Button {
                            label: "Close breakouts".to_string(),
                            variant: design_system::ButtonVariant::Secondary,
                            size: design_system::ButtonSize::Sm,
                            on_click: move |_| props.on_close.call(()),
                        }
                    } else {
                        design_system::Button {
                            label: "Open breakouts".to_string(),
                            variant: design_system::ButtonVariant::Primary,
                            size: design_system::ButtonSize::Sm,
                            on_click: move |_| props.on_open.call(()),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str, name: &str, members: &[&str]) -> BreakoutRoomView {
        BreakoutRoomView {
            id: id.into(),
            name: name.into(),
            members: members.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn on_opened_then_closed_round_trips() {
        let mut s = BreakoutClientState::default();
        assert!(!s.open);
        s.on_opened(vec![room("r1", "Group 1", &["u1", "u2"])]);
        assert!(s.open);
        assert_eq!(s.rooms.len(), 1);
        assert_eq!(s.assigned_count(), 2);
        s.on_closed();
        assert!(!s.open);
        assert!(s.rooms.is_empty());
        assert!(s.my_room.is_none());
    }

    #[test]
    fn on_updated_replaces_rooms_while_open() {
        let mut s = BreakoutClientState::default();
        s.on_opened(vec![room("r1", "A", &["u1"])]);
        s.on_updated(vec![room("r1", "A", &["u1", "u2"]), room("r2", "B", &[])]);
        assert_eq!(s.rooms.len(), 2);
        assert_eq!(s.assigned_count(), 2);
    }

    #[test]
    fn on_snapshot_hydrates_open_and_clears_when_closed() {
        let mut s = BreakoutClientState::default();
        s.on_snapshot(true, vec![room("r1", "A", &["u1"])]);
        assert!(s.open);
        assert_eq!(s.rooms.len(), 1);
        s.on_snapshot(false, vec![room("r9", "X", &["u9"])]);
        assert!(!s.open);
        assert!(s.rooms.is_empty());
    }

    #[test]
    fn on_my_assignment_sets_and_clears_room() {
        let mut s = BreakoutClientState::default();
        s.on_opened(vec![room("r1", "Group 1", &["u1"])]);

        let url = s.on_my_assignment(
            Some("r1".into()),
            "Group 1".into(),
            "http://webrtc.example/aula/t/c/s/breakout/r1/whep".into(),
        );
        assert_eq!(
            url.as_deref(),
            Some("http://webrtc.example/aula/t/c/s/breakout/r1/whep")
        );
        assert_eq!(s.my_room.as_ref().unwrap().room_name, "Group 1");

        // Back to main room → cleared, no url.
        let url = s.on_my_assignment(None, String::new(), String::new());
        assert!(url.is_none());
        assert!(s.my_room.is_none());
    }

    #[test]
    fn banner_hidden_when_closed_and_shows_room_when_assigned() {
        // Closed → renders nothing.
        fn closed_app() -> Element {
            rsx! { BreakoutBanner { state: BreakoutClientState::default() } }
        }
        let mut vdom = VirtualDom::new(closed_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("breakout-banner"),
            "should be hidden: {html}"
        );

        // Open + assigned → shows the room name.
        fn assigned_app() -> Element {
            let mut s = BreakoutClientState::default();
            s.on_opened(vec![room("r1", "Group 1", &["u1"])]);
            s.on_my_assignment("r1".to_string().into(), "Group 1".into(), "u".into());
            rsx! { BreakoutBanner { state: s } }
        }
        let mut vdom = VirtualDom::new(assigned_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Group 1"), "room name missing: {html}");
        assert!(
            html.contains("breakout-banner-row--assigned"),
            "assigned row missing: {html}"
        );
    }

    #[test]
    fn panel_shows_setup_when_no_rooms_and_list_when_rooms_exist() {
        fn setup_app() -> Element {
            rsx! {
                BreakoutPanel {
                    state: BreakoutClientState::default(),
                    participants: vec![("u1".into(), "Ada".into())],
                    on_auto_split: move |_| {},
                    on_create: move |_| {},
                    on_assign: move |_| {},
                    on_open: move |_| {},
                    on_close: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(setup_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Auto-split"), "auto-split missing: {html}");

        fn list_app() -> Element {
            let mut s = BreakoutClientState::default();
            s.on_opened(vec![room("r1", "Group 1", &["u1"])]);
            rsx! {
                BreakoutPanel {
                    state: s,
                    participants: vec![("u1".into(), "Ada".into())],
                    on_auto_split: move |_| {},
                    on_create: move |_| {},
                    on_assign: move |_| {},
                    on_open: move |_| {},
                    on_close: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(list_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Group 1"), "room list missing: {html}");
        assert!(
            html.contains("Close breakouts"),
            "close control missing (open): {html}"
        );
        assert!(html.contains("Ada"), "participant name missing: {html}");
    }
}
