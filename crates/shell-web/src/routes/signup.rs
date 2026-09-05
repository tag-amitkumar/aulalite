// crates/shell-web/src/routes/signup.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{self, ApiContext};

use crate::contexts::{UserContext, UserContextSignal};
use crate::route_enum::Route;

#[component]
pub fn Signup() -> Element {
    let nav = use_navigator();
    let mut api_signal = use_context::<Signal<ApiContext>>();
    let mut user_signal = use_context::<UserContextSignal>();
    let mut provisioning_error = use_signal(|| None::<String>);

    let on_success = move |token: String| {
        provisioning_error.set(None);
        #[cfg(target_arch = "wasm32")]
        let base_url = api::web_api_base_url();
        #[cfg(not(target_arch = "wasm32"))]
        let base_url = api::native_api_base_url();
        api_signal.set(ApiContext {
            base_url,
            id_token: token,
        });
        let api_ctx = api_signal.read().clone();
        spawn(async move {
            match api::get_me(&api_ctx).await {
                Ok(dto) if dto.tenant_id.is_some() => {
                    let destination =
                        if matches!(dto.tenant_role.as_deref(), Some("org_owner" | "org_admin")) {
                            Route::Onboarding {}
                        } else {
                            // Invitation-based signups join the academy they were
                            // invited to and must not be sent through owner setup.
                            Route::Dashboard {}
                        };
                    user_signal.set(Some(UserContext::from_dto(dto)));
                    nav.push(destination);
                }
                Ok(_) => {
                    provisioning_error.set(Some(
                        "Your account is ready, but public workspace creation is disabled. Ask an academy administrator for an invitation, then sign in again."
                            .into(),
                    ));
                }
                Err(err) => {
                    provisioning_error.set(Some(provisioning_failure_message(&err)));
                }
            }
        });
    };

    rsx! {
        div { class: "auth-composite motion-page",
            section { class: "auth-hero-visual",
                design_system::AuthHero {}
                div { class: "auth-hero-copy",
                    div { class: "auth-hero-brand",
                        // mark.svg is a dark emblem with a cream "A" — reads cleanly on the warm canvas
                        img {
                            class: "auth-hero-mark",
                            src: "/assets/brand/aulalite-mark.svg",
                            alt: "AulaLite",
                        }
                        // serif wordmark drawn in CSS — dark green with a gold "Lite" on the cream stage
                        span { class: "auth-hero-wordmark", "Aula", span { class: "gold", "Lite" } }
                    }
                    p { class: "auth-hero-kicker", "Start your academy" }
                    h2 { "Spin up a live academy in minutes." }
                    p { "Create courses, host live rooms, and bring your students into one polished workspace." }
                    div { class: "auth-hero-meta",
                        span { strong { "No setup" } " required" }
                        span { strong { "Live rooms" } " out of the box" }
                    }
                }
            }
            div { class: "auth-panel-stack",
                features_auth::Signup { on_success: on_success }
                if let Some(message) = provisioning_error.read().as_ref() {
                    div { class: "ds-form-error", role: "alert",
                        p { "{message}" }
                        a { class: "auth-link", href: "/login", "Go to sign in" }
                    }
                }
            }
        }
    }
}

fn provisioning_failure_message(err: &api::ApiError) -> String {
    let detail = err.to_string();
    if detail.contains("email_verification_required") {
        "Verify your email using the link we sent. Keep this page open; setup will continue automatically after verification. If you already closed it, verify first and then sign in."
            .into()
    } else if detail.contains("email_required") {
        "Firebase did not provide an email address for this account. Use an email-based account or contact support."
            .into()
    } else {
        "We created your sign-in, but could not finish setting up your academy. Please try signing in again; if the problem continues, contact support."
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::provisioning_failure_message;
    use features_courses::api::ApiError;

    #[test]
    fn verification_failure_has_an_actionable_message() {
        let message = provisioning_failure_message(&ApiError::Status(
            401,
            r#"{"error":"email_verification_required"}"#.into(),
        ));
        assert!(message.contains("Verify your email"));
        assert!(message.contains("sign in"));
    }
}
