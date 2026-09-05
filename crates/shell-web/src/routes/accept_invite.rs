// crates/shell-web/src/routes/accept_invite.rs
use dioxus::prelude::*;
use features_courses::accept_invite::{AcceptInvite as AcceptInviteView, AcceptState};
use features_courses::api;

use crate::routes::use_api;

#[component]
pub fn AcceptInvite(token: String) -> Element {
    let api = use_api();
    let mut state = use_signal(|| AcceptState::Loading);

    let token_for_effect = token.clone();
    use_future(move || {
        let api = api.clone();
        let token = token_for_effect.clone();
        async move {
            // POST accept first.
            let redeemed = match api::accept_invitation(&api, &token).await {
                Ok(r) => r,
                Err(e) => {
                    state.set(AcceptState::Failure {
                        reason: format!("{e}"),
                    });
                    return;
                }
            };
            // Look up the course slug from /v1/me/courses to populate the Success link.
            let slug = match api::list_my_courses(&api).await {
                Ok(courses) => courses
                    .iter()
                    .find(|c| c.course_id == redeemed.course_id)
                    .map(|c| c.slug.clone())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            state.set(AcceptState::Success {
                course_title: redeemed.course_title,
                course_slug: slug,
            });
        }
    });

    rsx! {
        AcceptInviteView { state: state.read().clone() }
    }
}
