// crates/shell-web/src/routes/assignments_grade.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::submissions_grading_table::SubmissionsGradingTable;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentGrade(slug: String, id: String) -> Element {
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
    if !user.can_grade() {
        nav.push(Route::AssignmentDetail {
            slug: slug.clone(),
            id: id.clone(),
        });
        return rsx! { p { "Redirecting…" } };
    }

    let assignment = use_resource({
        let api = api.clone();
        let id = id.clone();
        move || {
            let api = api.clone();
            let id = id.clone();
            async move { api::get_assignment(&api, &id).await }
        }
    });

    let snap = assignment.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(a)) => rsx! {
            SubmissionsGradingTable {
                api: api.clone(),
                assignment: a.clone(),
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Assignment not found: {e}" } },
        None => rsx! { p { "Loading assignment…" } },
    };
    drop(snap);

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
            { body }
        }
    }
}
