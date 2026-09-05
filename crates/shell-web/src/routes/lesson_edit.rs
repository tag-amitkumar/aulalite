// crates/shell-web/src/routes/lesson_edit.rs
//
// LessonEdit route: loads the lesson by id (via course outline), renders
// `features_courses::lesson_editor::LessonEditor`, and persists changes via
// PATCH /v1/courses/:cid/modules/:mid/lessons/:lid.
use design_system::kinetics_ui::{Breadcrumb, BreadcrumbItem};
use design_system::{use_toast_sender, PageHeader, ToastLevel};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::lesson_editor::{LessonEditor, SaveLessonRequest};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn LessonEdit(slug: String, module_id: String, lesson_id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let toast = use_toast_sender();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };
    if !user.can_teach() {
        nav.replace(Route::CourseDetail { slug: slug.clone() });
        return rsx! { p { "This workspace role cannot edit lessons. Redirecting…" } };
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

    let outline = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                let course_snap = course.read_unchecked();
                let course_id = course_snap
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map(|c| c.id.clone());
                drop(course_snap);
                if let Some(cid) = course_id {
                    api::get_course_outline(&api, &cid).await
                } else {
                    Ok(Vec::new())
                }
            }
        }
    });

    let course_snap = course.read_unchecked();
    let outline_snap = outline.read_unchecked();
    // Breadcrumb / heading data from what's already fetched: the course title
    // once `course` resolves (slug until then), and the lesson title once the
    // outline resolves.
    let course_crumb = course_snap
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|c| c.title.clone())
        .unwrap_or_else(|| slug.clone());
    let lesson_title = outline_snap
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|modules| {
            modules
                .iter()
                .find(|m| m.id == module_id)
                .and_then(|m| m.lessons.iter().find(|l| l.id == lesson_id))
        })
        .map(|l| l.title.clone());
    let body: Element = match (course_snap.as_ref(), outline_snap.as_ref()) {
        (Some(Ok(c)), Some(Ok(modules))) => {
            let lesson = modules
                .iter()
                .find(|m| m.id == module_id)
                .and_then(|m| m.lessons.iter().find(|l| l.id == lesson_id))
                .cloned();
            match lesson {
                Some(l) => {
                    let course_id = c.id.clone();
                    let course_slug = slug.clone();
                    let mod_id = module_id.clone();
                    let les_id = lesson_id.clone();
                    // Prerequisite candidates: every other lesson in the course.
                    let sibling_lessons: Vec<features_courses::lesson_editor::SiblingLesson> =
                        modules
                            .iter()
                            .flat_map(|m| m.lessons.iter())
                            .filter(|sl| sl.id != l.id)
                            .map(|sl| features_courses::lesson_editor::SiblingLesson {
                                id: sl.id.clone(),
                                title: sl.title.clone(),
                            })
                            .collect();
                    rsx! {
                        LessonEditor {
                            lesson_id: l.id.clone(),
                            r#type: l.r#type.clone(),
                            title: l.title.clone(),
                            body_md: l.body_md.clone().unwrap_or_default(),
                            linked_session_id: l.live_session_id.clone(),
                            linked_video_asset_id: l.video_asset_id.clone(),
                            // Staff prerequisite + drip controls (the editor seeds
                            // current prerequisites on mount via the lessons API).
                            course_id: course_id.clone(),
                            module_id: mod_id.clone(),
                            sibling_lessons,
                            current_release_at: None,
                            // Available-sessions picker for `live_session`
                            // lessons is a follow-up that needs a separate
                            // "unscheduled sessions" backend listing.
                            available_sessions: Vec::new(),
                            on_save: {
                                let api = api.clone();
                                move |req: SaveLessonRequest| {
                                    let api = api.clone();
                                    let course_id = course_id.clone();
                                    let mod_id = mod_id.clone();
                                    let les_id = les_id.clone();
                                    let course_slug = course_slug.clone();
                                    let mut toast = toast;
                                    spawn(async move {
                                        let live_session_id = req
                                            .linked_session_id
                                            .as_deref()
                                            .map(Some);
                                        let body = api::PatchLessonBody {
                                            title: Some(req.title.as_str()),
                                            body_md: req.body_md.as_deref(),
                                            live_session_id,
                                        };
                                        match api::patch_lesson(
                                            &api,
                                            &course_id,
                                            &mod_id,
                                            &les_id,
                                            &body,
                                        )
                                        .await
                                        {
                                            Ok(_) => {
                                                toast.push(
                                                    ToastLevel::Success,
                                                    "Lesson saved",
                                                    "Changes are live.",
                                                );
                                                nav.push(Route::CourseDetail {
                                                    slug: course_slug.clone(),
                                                });
                                            }
                                            Err(e) => {
                                                toast.push(
                                                    ToastLevel::Danger,
                                                    "Save failed",
                                                    format!("{e}"),
                                                );
                                            }
                                        }
                                    });
                                }
                            },
                            on_video_uploaded: move |_asset_id: String| {
                                // Wiring the video editor's upload completion to
                                // the lesson is handled inside
                                // `LessonVideoEditor` itself via the file_assets
                                // API; nothing to do here.
                            },
                        }
                    }
                }
                None => rsx! { p { class: "error", "Lesson not found." } },
            }
        }
        (Some(Err(e)), _) => rsx! { p { class: "error", "{e}" } },
        (_, Some(Err(e))) => rsx! { p { class: "error", "{e}" } },
        _ => rsx! { p { "Loading…" } },
    };
    drop(course_snap);
    drop(outline_snap);

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
            div { class: "lesson-edit-page",
                Breadcrumb {
                    items: vec![
                        BreadcrumbItem::link("My Courses", "/courses"),
                        BreadcrumbItem::link(course_crumb, format!("/courses/{slug}")),
                        BreadcrumbItem::current("Edit lesson"),
                    ],
                    aria_label: "Lesson editor navigation".to_string(),
                }
                PageHeader {
                    title: lesson_title.unwrap_or_else(|| "Edit lesson".to_string()),
                    kicker: "Lesson editor".to_string(),
                }
                { body }
            }
        }
    }
}
