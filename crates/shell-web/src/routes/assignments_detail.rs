// crates/shell-web/src/routes/assignments_detail.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_detail::AssignmentDetail as AssignmentDetailView;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentDetail(slug: String, id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

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
            AssignmentDetailView {
                api: api.clone(),
                assignment_id: id.clone(),
                course_slug: slug.clone(),
                current_user_id: user.user_id.clone(),
                can_author: user.can_teach(),
                can_grade: user.can_grade(),
            }
        }
    }
}
