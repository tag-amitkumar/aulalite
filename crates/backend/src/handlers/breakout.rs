// crates/backend/src/handlers/breakout.rs
//! Live breakout rooms within a session.
//!
//! Breakouts are ephemeral (held in the live-room broker, mirroring polls /
//! presence / draw-permission) and driven entirely over the existing live-room
//! WebSocket — there are no HTTP routes and no DB table. The teacher issues
//! `create_breakouts` / `assign_breakout` / `open_breakouts` /
//! `close_breakouts` socket commands; this module owns the pure business logic
//! (validation, auto-split, MediaMTX path derivation, per-user assignment fan
//! out) plus the broker-mutation entry points the socket loop in
//! `handlers/live_sessions.rs` calls.
//!
//! The MediaMTX room key for a breakout is derived from the session's main path
//! plus the breakout id (`<main_path>/breakout/<breakout_id_simple>`), a
//! distinct path from the main room and the promoted-student paths. Assigned
//! students re-subscribe their WHEP viewer to this key; on close they fall back
//! to the main room. Read access is granted by the wildcard
//! `<main_path>/breakout/*` permission added to the viewer JWT at join time.

use uuid::Uuid;

use crate::services::live_room::{
    BreakoutRoom, BreakoutState, BrokerEvent, LiveRoomBroker, BREAKOUT_MAX_ROOMS,
    BREAKOUT_NAME_MAX_LEN,
};

/// Validation failures for a breakout mutation. Stringified into a
/// `CommandFailed.reason` by the socket arm (already user-facing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakoutError {
    /// Asked for zero rooms or more than `BREAKOUT_MAX_ROOMS`.
    RoomCount,
    /// A room name was blank or exceeded `BREAKOUT_NAME_MAX_LEN`.
    RoomName,
    /// Assignment referenced a room id that does not exist in the layout.
    UnknownRoom,
    /// A mutation other than create/open was attempted while no breakout layout
    /// exists (e.g. assign before create).
    NotInitialized,
}

impl BreakoutError {
    pub fn message(&self) -> String {
        match self {
            BreakoutError::RoomCount => {
                format!("a breakout session needs between 1 and {BREAKOUT_MAX_ROOMS} rooms")
            }
            BreakoutError::RoomName => format!(
                "each breakout room needs a name of at most {BREAKOUT_NAME_MAX_LEN} characters"
            ),
            BreakoutError::UnknownRoom => "that breakout room no longer exists".to_string(),
            BreakoutError::NotInitialized => "no breakout rooms have been created yet".to_string(),
        }
    }
}

/// MediaMTX path / room key for a breakout sub-room, derived from the session's
/// main path. Distinct from the main room and promoted-student paths.
pub fn breakout_path(main_path: &str, breakout_id: Uuid) -> String {
    format!(
        "{}/breakout/{}",
        main_path.trim_end_matches('/'),
        breakout_id.simple()
    )
}

/// Full WHEP subscribe URL for a breakout room, built from the public WebRTC
/// base (mirrors the promoted-student `whep_url`).
pub fn breakout_whep_url(public_webrtc_url: &str, main_path: &str, breakout_id: Uuid) -> String {
    format!(
        "{}/{}/whep",
        public_webrtc_url.trim_end_matches('/'),
        breakout_path(main_path, breakout_id)
    )
}

/// Build a fresh breakout layout from a list of room names. Validates the room
/// count and each name; mints a UUID per room. Rooms start empty (no members).
pub fn build_rooms(names: &[String]) -> Result<Vec<BreakoutRoom>, BreakoutError> {
    if names.is_empty() || names.len() > BREAKOUT_MAX_ROOMS {
        return Err(BreakoutError::RoomCount);
    }
    let mut rooms = Vec::with_capacity(names.len());
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.chars().count() > BREAKOUT_NAME_MAX_LEN {
            return Err(BreakoutError::RoomName);
        }
        rooms.push(BreakoutRoom {
            id: Uuid::new_v4(),
            name: trimmed.to_string(),
            members: Vec::new(),
        });
    }
    Ok(rooms)
}

/// Distribute `participants` round-robin across `room_count` auto-named rooms.
/// Used by the teacher's "auto-split" action. Returns the room layout.
pub fn auto_split(
    participants: &[Uuid],
    room_count: usize,
) -> Result<Vec<BreakoutRoom>, BreakoutError> {
    if room_count == 0 || room_count > BREAKOUT_MAX_ROOMS {
        return Err(BreakoutError::RoomCount);
    }
    let mut rooms: Vec<BreakoutRoom> = (0..room_count)
        .map(|i| BreakoutRoom {
            id: Uuid::new_v4(),
            name: format!("Group {}", i + 1),
            members: Vec::new(),
        })
        .collect();
    for (i, user) in participants.iter().enumerate() {
        rooms[i % room_count].members.push(*user);
    }
    Ok(rooms)
}

/// Move `user_id` into `room_id` within `state` (removing them from any other
/// room first). `room_id == None` returns the user to the main room. Returns an
/// error if the target room id is unknown.
pub fn assign(
    state: &mut BreakoutState,
    user_id: Uuid,
    room_id: Option<Uuid>,
) -> Result<(), BreakoutError> {
    if let Some(target) = room_id {
        if !state.rooms.iter().any(|r| r.id == target) {
            return Err(BreakoutError::UnknownRoom);
        }
    }
    // Remove from every room first so a user is in at most one breakout.
    for room in state.rooms.iter_mut() {
        room.members.retain(|m| *m != user_id);
    }
    if let Some(target) = room_id {
        if let Some(room) = state.rooms.iter_mut().find(|r| r.id == target) {
            room.members.push(user_id);
        }
    }
    Ok(())
}

/// The `BreakoutAssignment` event for a single user given the current layout.
/// When the user is unassigned (or breakouts are closed) the assignment points
/// back to the main room (`room_id = None`, empty key/url).
pub fn assignment_event(
    state: &BreakoutState,
    user_id: Uuid,
    main_path: &str,
    public_webrtc_url: &str,
) -> BrokerEvent {
    match state.room_for(user_id) {
        Some(room) => BrokerEvent::BreakoutAssignment {
            user_id,
            room_id: Some(room.id),
            room_key: breakout_path(main_path, room.id),
            whep_url: breakout_whep_url(public_webrtc_url, main_path, room.id),
            room_name: room.name.clone(),
        },
        None => BrokerEvent::BreakoutAssignment {
            user_id,
            room_id: None,
            room_key: String::new(),
            whep_url: String::new(),
            room_name: String::new(),
        },
    }
}

/// Every user currently assigned to any breakout room (deduped is unnecessary —
/// `assign` keeps each user in at most one room).
pub fn assigned_users(state: &BreakoutState) -> Vec<Uuid> {
    state
        .rooms
        .iter()
        .flat_map(|r| r.members.iter().copied())
        .collect()
}

/// Persist `state` to the broker and broadcast the appropriate layout event
/// (`BreakoutOpened` when transitioning into open, else `BreakoutUpdated`),
/// then fan a targeted `BreakoutAssignment` to every assigned user so each
/// re-subscribes their WHEP viewer to their room. The main-room participants do
/// not need an assignment unless they were previously in a breakout — callers
/// that move users out should pass them in `also_notify`.
pub async fn commit_and_broadcast(
    broker: &dyn LiveRoomBroker,
    session_id: Uuid,
    state: BreakoutState,
    just_opened: bool,
    main_path: &str,
    public_webrtc_url: &str,
    also_notify: &[Uuid],
) -> Result<(), String> {
    broker
        .breakout_set(session_id, state.clone())
        .await
        .map_err(|e| e.to_string())?;

    let layout = if just_opened {
        BrokerEvent::BreakoutOpened {
            rooms: state.rooms.clone(),
        }
    } else {
        BrokerEvent::BreakoutUpdated {
            rooms: state.rooms.clone(),
        }
    };
    let _ = broker.publish(session_id, layout).await;

    // Notify everyone currently assigned plus anyone the caller flagged (e.g.
    // a user just moved back to the main room).
    let mut targets = assigned_users(&state);
    for u in also_notify {
        if !targets.contains(u) {
            targets.push(*u);
        }
    }
    for user_id in targets {
        let evt = assignment_event(&state, user_id, main_path, public_webrtc_url);
        let _ = broker.publish(session_id, evt).await;
    }
    Ok(())
}

/// Close all breakouts: clear the layout, broadcast `BreakoutClosed`, and send
/// every previously-assigned user a main-room assignment so they re-subscribe
/// to the main feed.
pub async fn close_and_broadcast(
    broker: &dyn LiveRoomBroker,
    session_id: Uuid,
    prev: &BreakoutState,
) -> Result<(), String> {
    broker
        .breakout_set(session_id, BreakoutState::default())
        .await
        .map_err(|e| e.to_string())?;
    let _ = broker
        .publish(session_id, BrokerEvent::BreakoutClosed)
        .await;
    // Every previously-assigned user falls back to the main room.
    for user_id in assigned_users(prev) {
        let _ = broker
            .publish(
                session_id,
                BrokerEvent::BreakoutAssignment {
                    user_id,
                    room_id: None,
                    room_key: String::new(),
                    whep_url: String::new(),
                    room_name: String::new(),
                },
            )
            .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::live_room::MockLiveRoomBroker;

    const MAIN: &str = "aula/t/c/s";
    const WEBRTC: &str = "http://webrtc.example";

    #[test]
    fn breakout_path_is_distinct_from_student_path() {
        let id = Uuid::nil();
        let p = breakout_path(MAIN, id);
        assert_eq!(p, "aula/t/c/s/breakout/00000000000000000000000000000000");
        assert!(p.contains("/breakout/"));
        assert!(!p.contains("/student/"));
    }

    #[test]
    fn breakout_whep_url_built_from_public_base() {
        let id = Uuid::nil();
        let url = breakout_whep_url(WEBRTC, MAIN, id);
        assert!(url.starts_with("http://webrtc.example/aula/t/c/s/breakout/"));
        assert!(url.ends_with("/whep"));
    }

    #[test]
    fn build_rooms_validates_count_and_names() {
        assert_eq!(build_rooms(&[]), Err(BreakoutError::RoomCount));
        let too_many: Vec<String> = (0..BREAKOUT_MAX_ROOMS + 1)
            .map(|i| format!("r{i}"))
            .collect();
        assert_eq!(build_rooms(&too_many), Err(BreakoutError::RoomCount));
        assert_eq!(
            build_rooms(&["  ".to_string()]),
            Err(BreakoutError::RoomName)
        );
        let long = "x".repeat(BREAKOUT_NAME_MAX_LEN + 1);
        assert_eq!(build_rooms(&[long]), Err(BreakoutError::RoomName));

        let rooms = build_rooms(&["A".to_string(), " B ".to_string()]).unwrap();
        assert_eq!(rooms.len(), 2);
        assert_eq!(rooms[1].name, "B"); // trimmed
        assert!(rooms.iter().all(|r| r.members.is_empty()));
    }

    #[test]
    fn auto_split_distributes_round_robin() {
        let users: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        let rooms = auto_split(&users, 2).unwrap();
        assert_eq!(rooms.len(), 2);
        // 5 users across 2 rooms → 3 + 2.
        let total: usize = rooms.iter().map(|r| r.members.len()).sum();
        assert_eq!(total, 5);
        assert_eq!(rooms[0].members.len(), 3);
        assert_eq!(rooms[1].members.len(), 2);
        assert_eq!(auto_split(&users, 0), Err(BreakoutError::RoomCount));
    }

    #[test]
    fn assign_moves_user_to_at_most_one_room() {
        let mut state = BreakoutState {
            open: true,
            rooms: build_rooms(&["A".into(), "B".into()]).unwrap(),
        };
        let a = state.rooms[0].id;
        let b = state.rooms[1].id;
        let user = Uuid::new_v4();

        assign(&mut state, user, Some(a)).unwrap();
        assert!(state.rooms[0].members.contains(&user));

        // Re-assign moves, not duplicates.
        assign(&mut state, user, Some(b)).unwrap();
        assert!(!state.rooms[0].members.contains(&user));
        assert!(state.rooms[1].members.contains(&user));

        // Back to main room (None).
        assign(&mut state, user, None).unwrap();
        assert!(state.rooms.iter().all(|r| !r.members.contains(&user)));

        // Unknown room rejected.
        assert_eq!(
            assign(&mut state, user, Some(Uuid::new_v4())),
            Err(BreakoutError::UnknownRoom)
        );
    }

    #[test]
    fn assignment_event_points_to_room_or_main() {
        let mut state = BreakoutState {
            open: true,
            rooms: build_rooms(&["A".into()]).unwrap(),
        };
        let room_id = state.rooms[0].id;
        let user = Uuid::new_v4();
        assign(&mut state, user, Some(room_id)).unwrap();

        match assignment_event(&state, user, MAIN, WEBRTC) {
            BrokerEvent::BreakoutAssignment {
                room_id: rid,
                whep_url,
                ..
            } => {
                assert_eq!(rid, Some(room_id));
                assert!(whep_url.ends_with("/whep"));
            }
            other => panic!("wrong event: {other:?}"),
        }

        // Unassigned user → main room (None, empty url).
        let other_user = Uuid::new_v4();
        match assignment_event(&state, other_user, MAIN, WEBRTC) {
            BrokerEvent::BreakoutAssignment {
                room_id: rid,
                whep_url,
                ..
            } => {
                assert_eq!(rid, None);
                assert!(whep_url.is_empty());
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn commit_then_close_round_trips_broker_state() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let user = Uuid::new_v4();
        let mut state = BreakoutState {
            open: true,
            rooms: build_rooms(&["A".into(), "B".into()]).unwrap(),
        };
        let room = state.rooms[0].id;
        assign(&mut state, user, Some(room)).unwrap();

        commit_and_broadcast(&broker, session, state.clone(), true, MAIN, WEBRTC, &[])
            .await
            .unwrap();
        let stored = broker.breakout_get(session).await.unwrap();
        assert!(stored.open);
        assert_eq!(stored.room_for(user).map(|r| r.id), Some(room));

        close_and_broadcast(&broker, session, &stored)
            .await
            .unwrap();
        let after = broker.breakout_get(session).await.unwrap();
        assert!(!after.open);
        assert!(after.rooms.is_empty());
    }
}
