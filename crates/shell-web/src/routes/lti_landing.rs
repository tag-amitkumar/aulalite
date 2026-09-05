// crates/shell-web/src/routes/lti_landing.rs
//! Landing page for an LTI 1.3 launch.
//!
//! After verifying the launch and JIT-provisioning the user, the backend
//! (`/v1/lti/launch`) 302s the browser to
//! `{app_origin}/lti/landing?next=<same-origin-path>#token=<jwt>`. This page
//! mirrors `sso_finish`: it (wasm) reads the `#token=` fragment, stores it as the
//! bearer credential (`platform_bridge::web::persist_local_token`), populates the
//! live `ApiContext`, fetches `/v1/me`, then navigates to the `next` destination
//! (the resolved course, or `/courses`).
//!
//! The `next` path is produced server-side and is guaranteed same-origin
//! (open-redirect-safe); we additionally guard here that it begins with `/`.

use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::ApiContext;

// `UserContext` + `Route` are referenced only inside the wasm-only branch.
#[allow(unused_imports)]
use crate::contexts::UserContext;
use crate::contexts::UserContextSignal;
// Reuse the pure URL helpers from sso_finish (wasm-only, like its callers).
#[cfg(target_arch = "wasm32")]
use crate::routes::sso_finish::{clear_url_fragment, fragment_param_for, query_param_for};

#[component]
pub fn LtiLanding() -> Element {
    let api_signal = try_consume_context::<Signal<ApiContext>>();
    let user_signal = try_consume_context::<UserContextSignal>();
    let nav = use_navigator();

    let mut error = use_signal(|| Option::<String>::None);
    let mut started = use_signal(|| false);

    use_future(move || async move {
        if *started.read() {
            return;
        }
        started.set(true);

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(reason) = query_param_for("error") {
                error.set(Some(format!("Launch failed: {reason}")));
                return;
            }

            let token = match fragment_param_for("token") {
                Some(t) if !t.is_empty() => t,
                _ => {
                    error.set(Some("No launch token was returned.".into()));
                    return;
                }
            };

            // Replace the callback entry before persisting/navigating so the
            // browser Back button cannot reveal the bearer fragment.
            clear_url_fragment();

            // Same-origin destination resolved server-side; default to the
            // course list. Guard against anything that isn't an in-app path.
            let next = query_param_for("next")
                .filter(|p| safe_next_path(p))
                .unwrap_or_else(|| "/courses".to_string());

            platform_bridge::web::persist_local_token(&token);
            let ctx = ApiContext {
                base_url: api::web_api_base_url(),
                id_token: token,
            };
            if let Some(mut api_signal) = api_signal {
                api_signal.set(ctx.clone());
            }
            match api::get_me(&ctx).await {
                Ok(dto) => {
                    if let Some(mut user_signal) = user_signal {
                        user_signal.set(Some(UserContext::from_dto(dto)));
                    }
                    // Keep the launch inside this document. A full browser load
                    // would receive the app's normal frame-deny CSP and break an
                    // otherwise-valid LTI launch embedded by an LMS. Dioxus accepts
                    // an arbitrary internal path as a navigation target.
                    nav.replace(next);
                }
                Err(_) => {
                    platform_bridge::web::clear_local_token();
                    error.set(Some(
                        "We couldn't complete the launch. Please try again.".into(),
                    ));
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;

            let callback = match platform_bridge::deep_links::take_pending_auth_callback() {
                Some(callback)
                    if callback.kind == platform_bridge::deep_links::CallbackKind::Lti =>
                {
                    callback
                }
                _ => {
                    error.set(Some("No launch callback was returned.".into()));
                    return;
                }
            };
            if let Some(reason) = callback.error {
                error.set(Some(format!("Launch failed: {reason}")));
                return;
            }
            let Some(token) = callback.token else {
                error.set(Some("No launch token was returned.".into()));
                return;
            };
            if platform_bridge::native::NativeBridge::persist_backend_callback_token(&token)
                .is_err()
            {
                error.set(Some(
                    "The launch response was invalid. Please try again.".into(),
                ));
                return;
            }
            let ctx = ApiContext {
                base_url: api::native_api_base_url(),
                id_token: token,
            };
            if let Some(mut api_signal) = api_signal {
                api_signal.set(ctx.clone());
            }
            match api::get_me(&ctx).await {
                Ok(dto) => {
                    if let Some(mut user_signal) = user_signal {
                        user_signal.set(Some(UserContext::from_dto(dto)));
                    }
                    nav.replace(callback.next_path);
                }
                Err(_) => {
                    let _ = platform_bridge::native::NativeBridge.sign_out().await;
                    error.set(Some(
                        "We couldn't complete the launch. Please try again.".into(),
                    ));
                }
            }
        }
    });

    rsx! {
        div { class: "auth-composite motion-page",
            div { class: "auth-panel-stack",
                if let Some(message) = error.read().clone() {
                    div { class: "auth-panel", role: "alert",
                        h2 { "Launch failed" }
                        p { "{message}" }
                        a { href: "/login", class: "btn btn-primary", "Go to sign in" }
                    }
                } else {
                    div { class: "app-auth-splash", "aria-busy": "true",
                        span { class: "app-auth-splash-mark", "Aula", span { class: "gold", "Lite" } }
                        p { "Opening your course…" }
                    }
                }
            }
        }
    }
}

fn safe_next_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::safe_next_path;

    #[test]
    fn next_path_rejects_scheme_relative_and_backslash_redirects() {
        assert!(safe_next_path("/courses/algebra?tab=lessons"));
        assert!(!safe_next_path("//evil.example/path"));
        assert!(!safe_next_path("/\\evil.example/path"));
        assert!(!safe_next_path("https://evil.example/path"));
    }
}
