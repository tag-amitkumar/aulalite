// crates/shell-web/src/routes/course_list.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_list::{CourseList as CourseListView, CourseListItem};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn CourseList() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user_snapshot = user_ctx.read().clone();
    let user = match user_snapshot {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };
    let can_create = user.can_teach();

    let courses = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_courses(&api).await }
        }
    });

    let snap = courses.read_unchecked();
    let courses_state: Result<Vec<CourseListItem>, Option<String>> = match snap.as_ref() {
        Some(Ok(rows)) => Ok(rows
            .iter()
            .map(|c| CourseListItem {
                id: c.id.clone(),
                slug: c.slug.clone(),
                title: c.title.clone(),
                status: c.status.clone(),
                description: c.description.clone(),
                owner_user_id: c.owner_user_id.clone(),
                cover_asset_id: c.cover_asset_id.clone(),
            })
            .collect()),
        Some(Err(e)) => Err(Some(format!("{e}"))),
        None => Err(None),
    };
    drop(snap);

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    let list_body: Element = match courses_state {
        Ok(items) => rsx! {
            CourseListView {
                courses: items,
                can_create: can_create,
                on_create_clicked: move |_| { nav.push(Route::CourseNew {}); },
                is_teacher: user.is_course_staff(),
            }
        },
        Err(Some(e)) => rsx! { p { class: "error", "Could not load courses: {e}" } },
        Err(None) => rsx! { p { "Loading courses…" } },
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move {
                        let _ = platform_bridge::web::WebBridge.sign_out().await;
                    });
                }
                nav.push(Route::Login {});
            },
            { list_body }
        }
    }
}
