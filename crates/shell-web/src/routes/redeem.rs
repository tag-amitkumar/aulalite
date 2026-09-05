// crates/shell-web/src/routes/redeem.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::redeem_code::RedeemCode;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn Redeem() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut submitting = use_signal(|| false);

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let on_submit = move |code: String| {
        let api = api.clone();
        submitting.set(true);
        spawn(async move {
            match api::redeem_enrollment_code(&api, &code).await {
                Ok(r) => {
                    // Look up slug to navigate.
                    let slug = match api::list_my_courses(&api).await {
                        Ok(courses) => courses
                            .iter()
                            .find(|c| c.course_id == r.course_id)
                            .map(|c| c.slug.clone()),
                        Err(_) => None,
                    };
                    submitting.set(false);
                    match slug {
                        Some(s) => {
                            nav.push(Route::CourseDetail { slug: s });
                        }
                        None => {
                            nav.push(Route::Dashboard {});
                        }
                    }
                }
                Err(e) => {
                    submitting.set(false);
                    error.set(Some(format!("{e}")));
                }
            }
        });
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };
    let error_value = error.read().clone();

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            RedeemCode {
                on_submit: on_submit,
                submitting: *submitting.read(),
                error: error_value,
            }
        }
    }
}
