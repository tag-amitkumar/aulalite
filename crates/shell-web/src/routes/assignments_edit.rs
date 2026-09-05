// crates/shell-web/src/routes/assignments_edit.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_editor::AssignmentEditor;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentEdit(slug: String, id: String) -> Element {
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
    if !user.can_teach() {
        nav.replace(Route::AssignmentDetail {
            slug: slug.clone(),
            id: id.clone(),
        });
        return rsx! { p { "This workspace role cannot edit assignments. Redirecting…" } };
    }

    let course = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                let all = api::list_courses(&api).await?;
                all.into_iter()
                    .find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });
    let assignment = use_resource({
        let api = api.clone();
        let id = id.clone();
        move || {
            let api = api.clone();
            let id = id.clone();
            async move { api::get_assignment(&api, &id).await }
        }
    });

    let course_snap = course.read_unchecked();
    let assn_snap = assignment.read_unchecked();
    let body: Element = match (course_snap.as_ref(), assn_snap.as_ref()) {
        (Some(Ok(c)), Some(Ok(a))) => rsx! {
            AssignmentEditor {
                api: api.clone(),
                course_slug: slug.clone(),
                course_id: c.id.clone(),
                initial: Some(a.clone()),
            }
        },
        (Some(Err(e)), _) => rsx! { p { class: "error", "{e}" } },
        (_, Some(Err(e))) => rsx! { p { class: "error", "{e}" } },
        _ => rsx! { p { "Loading…" } },
    };
    drop(course_snap);
    drop(assn_snap);

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
