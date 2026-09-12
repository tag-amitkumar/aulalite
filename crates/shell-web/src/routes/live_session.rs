// crates/shell-web/src/routes/live_session.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{fetch_json, ApiError};
use features_courses::live_room_session::{LiveRoomSession, SessionConfig};
use features_courses::{CallerRole, LiveRoomShell, SessionStatus};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[derive(serde::Deserialize, Clone, Default, PartialEq)]
struct JoinResp {
    state: String,
    transport_mode: String,
    viewer_jwt: Option<String>,
    main_url: Option<String>,
    screen_url: Option<String>,
    instructor_user_id: Option<String>,
    course_title: String,
    scheduled_starts_at: String,
    #[serde(default)]
    has_recording: bool,
    /// STUN/TURN servers the backend selected for this session.
    ///
    /// This field used to be dropped on the floor, which silently capped every
    /// VIEWER at the hardcoded Google-STUN fallback in `live_room_whep`. A
    /// student on a different network than the teacher then had no relay
    /// candidate, so ICE found no path to MediaMTX and the video stayed black
    /// even though the WHEP handshake returned 201. The teacher side never hit
    /// this because the go-live path already installs its own list.
    #[serde(default)]
    ice_servers: Vec<features_courses::live_room_whep::IceServerConfig>,
}

/// How often a student sitting in the lobby re-asks whether class has started.
///
/// `POST /v1/sessions/{id}/join` is a POST by convention only -- `join_inner`
/// performs no INSERT/UPDATE/DELETE, so re-issuing it is a read. 5s keeps the
/// jump into the live room feeling immediate (a class starting is a human
/// action, not a machine event) at a cost of one small query per student per
/// 5s, and only while they are actually waiting.
const LOBBY_POLL_MS: u32 = 5_000;

/// Backoff after a transient failure, so a flaking backend is not hammered.
const LOBBY_POLL_BACKOFF_MS: u32 = 15_000;

/// How long to idle between visibility checks while the tab is in the
/// background.
///
/// A hidden tab issues NO requests. This is what replaced the old two-hour
/// `LOBBY_POLL_MAX_MS` deadline: the cap existed because a tab left open on a
/// class that never starts polled for as long as it lived, but a deadline
/// answered that by also cutting off students waiting on a teacher who was
/// merely running very late. Gating on whether anyone is actually looking
/// removes the waste without inventing a moment at which a waiting student
/// stops being told the truth.
#[cfg(target_arch = "wasm32")]
const LOBBY_POLL_HIDDEN_MS: u32 = 5_000;

/// Is this a session state this build actually understands?
///
/// `map_status` renders an unrecognised state as `Scheduled`, which is the
/// right RENDERING choice -- degrade to the lobby rather than a blank screen --
/// and the wrong POLLING choice. A state this build cannot interpret will never
/// be observed to "start", so a lobby keyed on it asks forever. That, not the
/// honest "teacher is late" case, is what the old two-hour cap was really
/// containing. Terminating on it directly lets the cap go.
fn is_known_state(s: &str) -> bool {
    matches!(s, "scheduled" | "live" | "ended" | "cancelled")
}

fn map_status(s: &str) -> SessionStatus {
    match s {
        "scheduled" => SessionStatus::Scheduled,
        "live" => SessionStatus::Live,
        "ended" => SessionStatus::Ended,
        "cancelled" => SessionStatus::Cancelled,
        _ => SessionStatus::Scheduled,
    }
}

#[component]
pub fn LiveSession(slug: String, session_id: String) -> Element {
    let nav = use_navigator();
    let user_ctx = use_user_context();
    let user_snap = user_ctx.read().clone();

    if user_snap.is_none() {
        nav.push(Route::Login {});
        return rsx! { p { "Redirecting…" } };
    }
    let user = user_snap.unwrap();

    let caller_role = if user.can_teach() {
        CallerRole::Teacher
    } else {
        CallerRole::Student
    };

    // Re-join on an interval while this participant is stuck in the lobby.
    //
    // `/join` is the ONLY endpoint that hands a student the viewer JWT and the
    // WHEP URL, and the backend mints those exclusively once the session is
    // `live` -- so a student who opened the room early holds `None` for both.
    // This used to be a one-shot `use_resource` whose closure read nothing
    // reactive, so it ran exactly once per mount: the room never learned that
    // class had begun and the student sat on "Waiting for the instructor…"
    // until they manually reloaded. There is no server push to lean on either
    // (`ServerEvent` has `SessionEnded` but no start event, and the socket is
    // owned by the View/Broadcast branches, which a lobby student never
    // mounts), so the lobby asks.
    //
    // Polling stops as soon as `should_poll_for_start` says this participant
    // is no longer looking at the lobby, which covers live, ended and
    // cancelled alike.
    let mut join_state: Signal<Option<Result<JoinResp, String>>> = use_signal(|| None);
    let session_id_for_poll = session_id.clone();
    let poll_role = caller_role.clone();
    // Re-read the context on every attempt rather than capturing one value:
    // a lobby wait can outlive the access token that was current at mount, and
    // a frozen token would 401 and evict a student who was waiting perfectly
    // happily.
    let api_signal = use_context::<Signal<features_courses::api::ApiContext>>();
    use_future(move || {
        let session_id = session_id_for_poll.clone();
        let poll_role = poll_role.clone();
        async move {
            // On wasm32 -- the target that actually serves students -- this is
            // a real loop: it awaits the poll interval and comes back, and the
            // hidden-tab branch below `continue`s. On host builds the body ends
            // in `return` by design, so an SSR render or a unit test runs one
            // iteration instead of spinning forever. Clippy only ever sees the
            // host cfg, where that reads as a loop that cannot loop.
            #[allow(clippy::never_loop)]
            loop {
                // A backgrounded tab asks nothing. Nobody is reading the lobby,
                // so a request now buys no one anything; when the tab comes
                // back we resume within one short idle. This is the whole
                // reason the poll no longer needs a deadline to be cheap.
                #[cfg(target_arch = "wasm32")]
                if features_courses::browser_runtime::page_is_hidden() {
                    gloo_timers::future::TimeoutFuture::new(LOBBY_POLL_HIDDEN_MS).await;
                    continue;
                }

                let api = api_signal.read().clone();
                let attempt = fetch_json::<JoinResp>(
                    &api,
                    "POST",
                    &format!("/v1/sessions/{session_id}/join"),
                    None::<&()>,
                )
                .await;

                let delay_ms = match attempt {
                    Ok(j) => {
                        let keep_waiting = features_courses::should_poll_for_start(
                            &poll_role,
                            &map_status(&j.state),
                        );
                        // A state this build cannot interpret is rendered as
                        // the lobby but will never be seen to start, so asking
                        // again is pointless rather than merely slow.
                        let understood = is_known_state(&j.state);
                        #[cfg(target_arch = "wasm32")]
                        if !understood {
                            web_sys::console::warn_1(
                                &format!(
                                    "[live_session] unrecognised session state {:?}; \
                                     stopping lobby poll",
                                    j.state
                                )
                                .into(),
                            );
                        }
                        join_state.set(Some(Ok(j)));
                        if !keep_waiting || !understood {
                            return;
                        }
                        // No elapsed-time bound. A class has no maximum
                        // duration and a teacher has no deadline to start by;
                        // the student waits until the class actually starts,
                        // is cancelled, or they close the tab.
                        LOBBY_POLL_MS
                    }
                    // An expired/rejected token will never recover by retrying,
                    // and silently looping on it would hide the sign-in prompt.
                    Err(ApiError::Status(401, body)) => {
                        join_state.set(Some(Err(ApiError::Status(401, body).to_string())));
                        return;
                    }
                    Err(e) => {
                        // Only surface a failure that stopped us getting in at
                        // all. Once a student is validly waiting, a transient
                        // blip must not evict them from the lobby.
                        //
                        // Read into a local first: holding the signal's borrow
                        // across the `set` below would be a RefCell double
                        // borrow.
                        let never_joined = join_state.read().is_none();
                        if never_joined {
                            join_state.set(Some(Err(e.to_string())));
                            return;
                        }
                        LOBBY_POLL_BACKOFF_MS
                    }
                };

                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(delay_ms).await;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Host builds (SSR/unit tests) run one iteration so a
                    // mounted route does not spin forever.
                    let _ = delay_ms;
                    return;
                }
            }
        }
    });

    let snap = join_state.read();
    let body: Element = match snap.as_ref() {
        Some(Ok(j)) => {
            // Install the session's ICE servers into the shared WHIP/WHEP
            // registry BEFORE the viewer subtree mounts. `LiveRoomView` builds
            // its WHEP PeerConnection during its own mount effect, which runs
            // after this parent render, so the relay list is in place by the
            // time the offer is created. Idempotent, so re-renders are cheap.
            features_courses::live_room_whep::set_ice_servers(j.ice_servers.clone());

            rsx! {
                LiveSessionShell {
                    session_id: session_id.clone(),
                    slug: slug.clone(),
                    caller_role: caller_role.clone(),
                    status: map_status(&j.state),
                    course_title: j.course_title.clone(),
                    instructor_name: j.instructor_user_id.clone(),
                    scheduled_starts_at_iso: j.scheduled_starts_at.clone(),
                    transport_mode: j.transport_mode.clone(),
                    viewer_jwt: j.viewer_jwt.clone(),
                    main_url: j.main_url.clone(),
                    screen_url: j.screen_url.clone(),
                    has_recording: j.has_recording,
                    is_teacher: user.is_course_staff(),
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Failed to join session: {e}" } },
        None => rsx! { p { "Joining…" } },
    };
    drop(snap);

    body
}

/// Wrapper component that owns the `LiveRoomSession` aggregate for the route.
///
/// * Builds a fresh `LiveRoomSession` once per mount and provides it via
///   `use_context_provider::<Signal<LiveRoomSession>>`.
/// * Registers a `use_drop` callback that awaits `session.close()` on route
///   exit so RTC peer connections, media tracks, and the WebSocket release
///   instead of leaking until the browser GCs them.
/// * Delegates rendering to `LiveRoomShell`, which picks the Broadcast / View
///   / Lobby / Replay branch as before.
#[component]
fn LiveSessionShell(
    session_id: String,
    slug: String,
    caller_role: CallerRole,
    status: SessionStatus,
    course_title: String,
    instructor_name: Option<String>,
    scheduled_starts_at_iso: String,
    transport_mode: String,
    viewer_jwt: Option<String>,
    main_url: Option<String>,
    screen_url: Option<String>,
    has_recording: bool,
    is_teacher: bool,
) -> Element {
    let api = use_api();

    // Build the SessionConfig from the join response + the api signal. The
    // `api_origin` mirrors the API base URL the bridge already resolved;
    // `access_token` is the current JWT held by ApiContext (the live signal
    // ensures the value here reflects the freshest token at construction).
    let config = SessionConfig {
        session_id: session_id.clone(),
        api_origin: api.base_url.clone(),
        access_token: api.id_token.clone(),
        viewer_jwt: viewer_jwt.clone(),
    };

    let session_signal: Signal<LiveRoomSession> =
        use_signal(|| LiveRoomSession::new(config.clone(), api.clone()));

    // Provide the session aggregate to descendant components (LiveRoomView,
    // LiveRoomBroadcast). They read it via `try_consume_context` and route
    // their socket / WHIP / WHEP work through the session.
    use_context_provider(|| session_signal);

    // Awaited cleanup on route exit. `use_drop` runs at component unmount
    // (which happens on route change). We spawn a wasm task that takes the
    // session out of the signal and awaits `close()` so the WebSocket and
    // RTC PeerConnections release deterministically.
    let session_for_drop = session_signal;
    use_drop(move || {
        #[cfg(target_arch = "wasm32")]
        {
            let mut sig = session_for_drop;
            wasm_bindgen_futures::spawn_local(async move {
                sig.write().close().await;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = session_for_drop;
        }
    });

    rsx! {
        LiveRoomShell {
            session_id: session_id.clone(),
            course_slug: slug.clone(),
            caller_role: caller_role,
            status: status,
            course_title: course_title.clone(),
            instructor_name: instructor_name.clone(),
            scheduled_starts_at_iso: scheduled_starts_at_iso.clone(),
            transport_mode: transport_mode.clone(),
            viewer_jwt: viewer_jwt.clone(),
            main_url: main_url.clone(),
            screen_url: screen_url.clone(),
            has_recording: has_recording,
            is_teacher: is_teacher,
        }
    }
}

#[cfg(test)]
mod tests {
    // `CallerRole` and `SessionStatus` arrive via the glob from the parent.
    use super::*;
    use features_courses::should_poll_for_start;

    #[test]
    fn a_lobby_keeps_waiting_however_late_the_teacher_is() {
        // Regression: the poll used to stop after a fixed two hours, so a
        // student waiting on a class that started late sat on "Waiting for the
        // instructor..." forever even once it went live. Nothing about the
        // decision to keep waiting may depend on elapsed time -- it is a pure
        // function of the session's state.
        assert!(should_poll_for_start(
            &CallerRole::Student,
            &map_status("scheduled")
        ));
    }

    #[test]
    fn a_lobby_stops_once_the_session_reaches_a_real_outcome() {
        // The poll is bounded by events, not by a clock.
        for terminal in ["live", "ended", "cancelled"] {
            assert!(
                !should_poll_for_start(&CallerRole::Student, &map_status(terminal)),
                "{terminal} must stop the lobby poll"
            );
        }
    }

    #[test]
    fn an_unrecognised_state_is_rendered_as_the_lobby_but_never_polled_forever() {
        // `map_status` degrades an unknown state to the lobby so the UI is not
        // blank -- which on its own would poll forever, since such a state can
        // never be observed to start. That combination, not the honest "late
        // teacher" case, is what the old cap was containing, so the poll
        // terminates on it explicitly instead.
        assert_eq!(map_status("something_new"), SessionStatus::Scheduled);
        assert!(should_poll_for_start(
            &CallerRole::Student,
            &map_status("something_new")
        ));
        assert!(!is_known_state("something_new"));

        for known in ["scheduled", "live", "ended", "cancelled"] {
            assert!(is_known_state(known), "{known} must be understood");
        }
    }
}
