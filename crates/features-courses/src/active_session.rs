// crates/features-courses/src/active_session.rs
//! Hook that polls `/v1/courses/:cid/active-session` while mounted so the
//! teacher's start-now button and the student's "Live now" banner can react
//! within ~15s of a teacher hitting Start.
//!
//! Backoff: success → 15s; transient failure (network / 5xx) → 30s; 401
//! short-circuits and stops polling (the ApiContext refresh path resumes
//! on next user interaction).

use crate::api::{self, ActiveSessionInfoDto, ApiContext, ApiError};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum PollState {
    Loading,
    Active(ActiveSessionInfoDto),
    Idle,
    Stopped,
}

/// Polls `/active-session` for `course_id` every 15s on success, 30s after
/// a transient error. Returns a Signal that components can read.
///
/// The poll loop terminates when the component using this hook unmounts
/// (dioxus drops the scoped future).
#[allow(clippy::never_loop)]
pub fn use_active_session_poll(course_id: String) -> Signal<PollState> {
    let api = use_context::<Signal<ApiContext>>();
    let mut state = use_signal(|| PollState::Loading);

    use_future(move || {
        let course_id = course_id.clone();
        let api = api;
        async move {
            loop {
                // Hidden tabs do not need to hit the backend. Keep the normal
                // cadence so returning users see fresh state promptly without
                // every background course tab generating poll traffic.
                #[cfg(target_arch = "wasm32")]
                if crate::browser_runtime::page_is_hidden() {
                    gloo_timers::future::TimeoutFuture::new(15_000).await;
                    continue;
                }

                let ctx = api.read().clone();
                let delay_ms = match api::get_active_session(&ctx, &course_id).await {
                    Ok(resp) => {
                        let new_state = match resp.active {
                            Some(info) => PollState::Active(info),
                            None => PollState::Idle,
                        };
                        state.set(new_state);
                        15_000_u32
                    }
                    Err(ApiError::Status(401, _)) => {
                        state.set(PollState::Stopped);
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "active_session poll failed; backing off 30s"
                        );
                        30_000_u32
                    }
                };
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(delay_ms).await;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Host build: the poll loop is wasm-only in practice.
                    // Return after one iteration so tests that mount the
                    // hook don't spin.
                    let _ = delay_ms;
                    return;
                }
            }
        }
    });

    state
}
