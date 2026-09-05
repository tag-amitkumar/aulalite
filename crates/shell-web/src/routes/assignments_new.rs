// crates/shell-web/src/routes/assignments_new.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_editor::AssignmentEditor;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentNew(slug: String) -> Element {
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
        nav.replace(Route::AssignmentList { slug: slug.clone() });
        return rsx! { p { "This workspace role cannot create assignments. Redirecting…" } };
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

    let snap = course.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(c)) => rsx! {
            design_system::kinetics_ui::Breadcrumb {
                items: vec![
                    design_system::kinetics_ui::BreadcrumbItem::link("My Courses", "/courses"),
                    design_system::kinetics_ui::BreadcrumbItem::link(c.title.clone(), format!("/courses/{slug}")),
                    design_system::kinetics_ui::BreadcrumbItem::link("Assignments", format!("/courses/{slug}/assignments")),
                    design_system::kinetics_ui::BreadcrumbItem::current("New"),
                ],
                aria_label: "Assignment navigation".to_string(),
            }
            AssignmentEditor {
                api: api.clone(),
                course_slug: slug.clone(),
                course_id: c.id.clone(),
                initial: None,
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
        None => rsx! { p { "Loading course…" } },
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
