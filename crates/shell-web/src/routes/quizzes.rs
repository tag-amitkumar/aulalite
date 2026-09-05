// crates/shell-web/src/routes/quizzes.rs
//! Quiz routes (learning-suite Cycle 3): course quiz list, the student
//! taking flow, and the teacher editor.

use design_system::kinetics_ui::{Breadcrumb, BreadcrumbItem};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::quiz_editor::QuizEditor;
use features_courses::quiz_take::QuizTake;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Shared scaffold: resolve the course by slug, wrap in the AppShell, and
/// render `body(course)` once loaded. `leaf` is the trailing breadcrumb —
/// `None` marks the quiz list itself as current.
fn quiz_page(
    slug: String,
    leaf: Option<&'static str>,
    author_only: bool,
    learner_only: bool,
    body: impl Fn(&api::CourseDto) -> Element + 'static,
) -> Element {
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
    if author_only && !user.can_teach() {
        nav.replace(Route::QuizListPage { slug: slug.clone() });
        return rsx! { p { "This workspace role cannot edit quizzes. Redirecting…" } };
    }
    if learner_only && !user.can_learn() {
        nav.replace(Route::QuizListPage { slug: slug.clone() });
        return rsx! { p { "Quiz attempts are available to enrolled students. Redirecting…" } };
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
    let content: Element = match snap.as_ref() {
        Some(Ok(c)) => {
            let mut crumbs = vec![
                BreadcrumbItem::link("My Courses", "/courses"),
                BreadcrumbItem::link(c.title.clone(), format!("/courses/{slug}")),
            ];
            match leaf {
                Some(label) => {
                    crumbs.push(BreadcrumbItem::link(
                        "Quizzes",
                        format!("/courses/{slug}/quizzes"),
                    ));
                    crumbs.push(BreadcrumbItem::current(label));
                }
                None => crumbs.push(BreadcrumbItem::current("Quizzes")),
            }
            rsx! {
                Breadcrumb { items: crumbs, aria_label: "Quiz navigation".to_string() }
                {body(c)}
            }
        }
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
            { content }
        }
    }
}

#[component]
pub fn QuizListPage(slug: String) -> Element {
    // Thin delegate: the quiz list renders INSIDE the course shell (header +
    // tab bar) like every other tab; take/edit keep the focused scaffold.
    crate::routes::course_detail::render_with_tab(slug, "quizzes")
}

#[component]
pub fn QuizTakePage(slug: String, quiz_id: String) -> Element {
    let quiz_id_body = quiz_id.clone();
    quiz_page(slug, Some("Take quiz"), false, true, move |course| {
        rsx! {
            QuizTake {
                course_id: course.id.clone(),
                quiz_id: quiz_id_body.clone(),
            }
        }
    })
}

#[component]
pub fn QuizEditPage(slug: String, quiz_id: String) -> Element {
    let quiz_id_body = quiz_id.clone();
    quiz_page(slug, Some("Edit quiz"), true, false, move |course| {
        rsx! {
            QuizEditor {
                course_id: course.id.clone(),
                quiz_id: quiz_id_body.clone(),
            }
        }
    })
}
