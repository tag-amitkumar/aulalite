// crates/shell-web/src/routes/course_new.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{self, CreateCourseBody};
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_create::CourseCreate;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn CourseNew() -> Element {
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
        nav.replace(Route::CourseList {});
        return rsx! { p { "This workspace role cannot create courses. Redirecting…" } };
    }

    let create_fn =
        move |(title, description, cb): (String, String, EventHandler<Result<String, String>>)| {
            let api = api.clone();
            spawn(async move {
                let body = CreateCourseBody {
                    title: &title,
                    description: if description.is_empty() {
                        None
                    } else {
                        Some(&description)
                    },
                };
                match api::create_course(&api, &body).await {
                    Ok(c) => cb.call(Ok(c.slug)),
                    Err(e) => cb.call(Err(format!("{e}"))),
                }
            });
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
                    spawn(async move {
                        let _ = platform_bridge::web::WebBridge.sign_out().await;
                    });
                }
                nav.push(Route::Login {});
            },
            CourseCreate {
                on_created: move |slug: String| { nav.push(Route::CourseDetail { slug }); },
                create_fn: create_fn,
            }
        }
    }
}
