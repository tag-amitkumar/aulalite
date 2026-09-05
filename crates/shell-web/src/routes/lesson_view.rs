// crates/shell-web/src/routes/lesson_view.rs
//! Student lesson page (learning-suite Cycle 2): renders one lesson's
//! content next to the progress-aware course outline, with an explicit
//! "Mark as complete" action and previous/next navigation.

use design_system::kinetics_ui::{Breadcrumb, BreadcrumbItem};
use design_system::{Button, ButtonVariant, SkeletonCard};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_progress::StudentCourseOutline;
use features_courses::lesson_outline_view::{LessonOutlineView, LessonView};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn LessonPage(slug: String, lesson_id: String) -> Element {
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

    let course_resource = use_resource({
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

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    let snap = course_resource.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(course)) => rsx! {
            LessonPageBody {
                course_id: course.id.clone(),
                course_title: course.title.clone(),
                slug: slug.clone(),
                lesson_id: lesson_id.clone(),
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Course not found: {e}" } },
        None => rsx! { SkeletonCard {} },
    };
    drop(snap);

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
            { body }
        }
    }
}

#[component]
fn LessonPageBody(
    course_id: String,
    course_title: String,
    slug: String,
    lesson_id: String,
) -> Element {
    let nav = use_navigator();
    let api = use_api();

    let modules = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&api, &course_id).await }
        }
    });
    let mut progress = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_progress(&api, &course_id).await }
        }
    });
    // Published quizzes feed the end-of-course handoff CTA. Best-effort: a
    // failure just hides that branch.
    let quizzes = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_quizzes(&api, &course_id).await }
        }
    });

    let modules_snap = modules.read_unchecked();
    let progress_snap = progress.read_unchecked();
    let (Some(Ok(modules_data)), Some(Ok(progress_data))) =
        (modules_snap.as_ref(), progress_snap.as_ref())
    else {
        if let Some(Err(e)) = modules_snap.as_ref() {
            return rsx! { p { class: "error", "Could not load the course outline: {e}" } };
        }
        if let Some(Err(e)) = progress_snap.as_ref() {
            return rsx! { p { class: "error", "Could not load progress: {e}" } };
        }
        return rsx! { SkeletonCard {} };
    };

    // Flattened lesson order (module order, then lesson order) for prev/next.
    let ordered: Vec<&api::LessonSummaryDto> =
        modules_data.iter().flat_map(|m| m.lessons.iter()).collect();
    let position = ordered.iter().position(|l| l.id == lesson_id);
    let Some(position) = position else {
        return rsx! { p { class: "error", "Lesson not found in this course." } };
    };
    let lesson = ordered[position];
    let prev_id = position.checked_sub(1).map(|i| ordered[i].id.clone());
    let next_id = ordered.get(position + 1).map(|l| l.id.clone());
    let next_title = ordered.get(position + 1).map(|l| l.title.clone());
    let is_completed = progress_data.completed_lesson_ids.contains(&lesson_id);
    let course_done = progress_data.total > 0 && progress_data.completed >= progress_data.total;
    let published_quiz_count: usize = {
        let snap = quizzes.read_unchecked();
        match snap.as_ref() {
            Some(Ok(list)) => list.len(),
            _ => 0,
        }
    };

    let lesson_view = LessonView {
        id: lesson.id.clone(),
        r#type: lesson.r#type.clone(),
        title: lesson.title.clone(),
        body_md: lesson.body_md.clone(),
        video_asset_id: lesson.video_asset_id.clone(),
        live_session_id: lesson.live_session_id.clone(),
    };

    let on_toggle_complete = {
        let api = api.clone();
        let course_id = course_id.clone();
        let lesson_id = lesson_id.clone();
        move |_| {
            let api = api.clone();
            let course_id = course_id.clone();
            let lesson_id = lesson_id.clone();
            let next = !is_completed;
            spawn(async move {
                if api::set_lesson_completion(&api, &course_id, &lesson_id, next)
                    .await
                    .is_ok()
                {
                    progress.restart();
                }
            });
        }
    };

    let nav_to_lesson = {
        let slug = slug.clone();
        move |target: String| {
            nav.push(Route::LessonPage {
                slug: slug.clone(),
                lesson_id: target,
            });
        }
    };
    let has_prev = prev_id.is_some();
    let has_next = next_id.is_some();
    let nav_prev = {
        let nav_to = nav_to_lesson.clone();
        let prev_id = prev_id.clone();
        move |_| {
            if let Some(id) = prev_id.clone() {
                nav_to(id);
            }
        }
    };
    let nav_next = {
        let nav_to = nav_to_lesson.clone();
        let next_id = next_id.clone();
        move |_| {
            if let Some(id) = next_id.clone() {
                nav_to(id);
            }
        }
    };
    let on_outline_select = {
        let nav_to = nav_to_lesson.clone();
        move |id: String| nav_to(id)
    };

    // Post-completion continuation: the locked flow is CTA-driven (never
    // auto-navigate). Next lesson → quizzes handoff → course-complete state.
    let continue_cta: Element = if !is_completed {
        rsx! {}
    } else if let Some(title) = next_title.clone() {
        let nav_next_cta = {
            let nav_to = nav_to_lesson.clone();
            let next_id = next_id.clone();
            move |_| {
                if let Some(id) = next_id.clone() {
                    nav_to(id);
                }
            }
        };
        rsx! {
            div { class: "lesson-continue",
                span { class: "lesson-continue-kicker", "Up next" }
                Button {
                    label: format!("Next: {title} →"),
                    variant: ButtonVariant::Primary,
                    on_click: nav_next_cta,
                }
            }
        }
    } else if published_quiz_count > 0 {
        rsx! {
            div { class: "lesson-continue",
                span { class: "lesson-continue-kicker", "All lessons done" }
                a { class: "ds-button ds-button--primary", href: "/courses/{slug}/quizzes",
                    if published_quiz_count == 1 { "Continue to the quiz →" } else { "Continue to quizzes →" }
                }
            }
        }
    } else if course_done {
        rsx! {
            div { class: "lesson-continue lesson-continue--complete",
                span { class: "lesson-continue-kicker", "🎉 Course complete" }
                p { class: "lesson-continue-copy",
                    "You've finished every lesson. Your teacher can now issue your certificate."
                }
                a { class: "ds-button ds-button--secondary", href: "/courses/{slug}/certificates",
                    "View certificate status →"
                }
            }
        }
    } else {
        rsx! {}
    };

    let lesson_number = position + 1;
    let lesson_count = ordered.len();
    let done = progress_data.completed;
    let total = progress_data.total;

    rsx! {
        div { class: "lesson-page lesson-reading-page motion-page",
            Breadcrumb {
                items: vec![
                    BreadcrumbItem::link("My Courses", "/courses"),
                    BreadcrumbItem::link(course_title.clone(), format!("/courses/{slug}")),
                    BreadcrumbItem::current(lesson.title.clone()),
                ],
                aria_label: "Lesson navigation".to_string(),
            }
            div { class: "lesson-reading-progress",
                span { class: "lesson-reading-progress-label",
                    "Lesson {lesson_number} of {lesson_count} · {done}/{total} completed"
                }
                div {
                    class: "lesson-reading-progress-track",
                    role: "progressbar",
                    "aria-valuemin": "0",
                    "aria-valuemax": "{total}",
                    "aria-valuenow": "{done}",
                    div {
                        class: "lesson-reading-progress-fill",
                        style: if total > 0 {
                            format!("width: {}%", (done * 100) / total)
                        } else {
                            "width: 0%".to_string()
                        },
                    }
                }
            }
            div { class: "lesson-page-grid lesson-reading-grid",
                main { class: "lesson-page-main",
                    article { class: "lesson-reading",
                        LessonOutlineView { lesson: lesson_view, course_slug: Some(slug.clone()) }
                    }
                    { continue_cta }
                    features_courses::lesson_notes::LessonNotes { lesson_id: lesson_id.clone() }
                    div { class: "lesson-page-actions",
                        Button {
                            label: if is_completed { "Completed ✓ — undo".to_string() } else { "Mark as complete".to_string() },
                            variant: if is_completed { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                            on_click: on_toggle_complete,
                        }
                        div { class: "lesson-page-pager",
                            Button {
                                label: "← Previous".to_string(),
                                variant: ButtonVariant::Ghost,
                                disabled: !has_prev,
                                on_click: nav_prev,
                            }
                            Button {
                                label: "Next →".to_string(),
                                variant: ButtonVariant::Ghost,
                                disabled: !has_next,
                                on_click: nav_next,
                            }
                        }
                    }
                }
                aside { class: "lesson-page-side lesson-reading-rail",
                    StudentCourseOutline {
                        modules: modules_data.clone(),
                        progress: progress_data.clone(),
                        on_select: on_outline_select,
                    }
                }
            }
        }
    }
}
