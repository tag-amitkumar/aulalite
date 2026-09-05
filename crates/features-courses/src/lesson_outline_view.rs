// crates/features-courses/src/lesson_outline_view.rs
//! Student-side display: type-switches a single lesson into rich_text /
//! video / live_session / file_bundle render branches.

use crate::api::{self, fetch_json, ApiContext};
use crate::file_asset_image::FileAssetImage;
use design_system::{Card, FileCard};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct LessonView {
    pub id: String,
    pub r#type: String, // 'rich_text' | 'video' | 'live_session' | 'file_bundle'
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<String>,
    pub live_session_id: Option<String>,
}

#[derive(serde::Deserialize, Clone)]
struct LessonFile {
    asset_id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(serde::Deserialize)]
struct UrlResponse {
    url: String,
}

/// Optional drip/prerequisite lock reason. When `Some`, the lesson body is
/// replaced by a locked notice. Callers compute this from
/// `GET /v1/courses/:cid/lessons/lock-state` (see `course_progress.rs`) and
/// pass it in — kept as a prop (not self-fetched) so this component does not
/// add conditional hooks that could violate Dioxus's rules of hooks.
#[derive(Props, Clone, PartialEq)]
pub struct LessonOutlineViewProps {
    pub lesson: LessonView,
    /// Course slug for building the join-session link
    /// (`/courses/{slug}/sessions/{id}`). Optional so older call sites keep
    /// compiling; without it the live-session CTA renders without a link.
    #[props(default)]
    pub course_slug: Option<String>,
    /// When set, the lesson is locked for the caller and the body is replaced
    /// by a notice carrying this reason (drip date / unmet prerequisites).
    #[props(default)]
    pub locked_reason: Option<String>,
}

#[component]
pub fn LessonOutlineView(props: LessonOutlineViewProps) -> Element {
    let lesson = props.lesson.clone();
    let cx = api::use_api();

    if let Some(reason) = props.locked_reason.clone() {
        return rsx! {
            Card {
                h3 { "{lesson.title}" }
                div { class: "lesson-locked-notice",
                    span { class: "lesson-locked-notice-icon", "🔒" }
                    div {
                        p { class: "lesson-locked-notice-title", "This lesson is locked" }
                        p { class: "lesson-locked-notice-reason", "{reason}" }
                    }
                }
            }
        };
    }

    rsx! {
        Card {
            h3 { "{lesson.title}" }
            match lesson.r#type.as_str() {
                "rich_text" => rsx! {
                    {render_markdown(lesson.body_md.as_deref().unwrap_or(""))}
                },
                "video" => rsx! {
                    {render_video(&lesson, &cx)}
                },
                "live_session" => rsx! {
                    {render_live_session(&lesson, props.course_slug.as_deref())}
                },
                "file_bundle" => rsx! {
                    {render_file_bundle(&lesson, &cx)}
                },
                _ => rsx! { p { "Unknown lesson type." } },
            }
        }
    }
}

/// Live-session branch: a designed join block instead of a raw status word.
/// With a `course_slug` the CTA is a prominent link-button to the live room;
/// without one (older call sites) the styled block renders link-less.
fn render_live_session(lesson: &LessonView, course_slug: Option<&str>) -> Element {
    let Some(sid) = lesson.live_session_id.as_deref() else {
        return rsx! {
            p { class: "live-session-pointer muted", "Live session not yet scheduled." }
        };
    };
    let href = course_slug.map(|slug| format!("/courses/{slug}/sessions/{sid}"));
    rsx! {
        div { class: "live-session-cta",
            div { class: "live-session-cta-copy",
                span { class: "live-session-cta-title", "This lesson meets live" }
                p { class: "live-session-cta-sub",
                    "Head to the live classroom to take part — video, chat, and the whiteboard are all in the room."
                }
            }
            if let Some(href) = href {
                a { class: "ds-button ds-button--primary live-session-cta-join", href: "{href}",
                    "Join live session"
                }
            }
        }
    }
}

fn render_markdown(body: &str) -> Element {
    let out = crate::markdown::render_user_markdown_html(body);
    rsx! {
        div { class: "lesson-md", dangerous_inner_html: "{out}" }
    }
}

fn render_video(lesson: &LessonView, _cx: &ApiContext) -> Element {
    if let Some(asset_id) = &lesson.video_asset_id {
        let asset_id = asset_id.clone();
        let cx_inner = api::use_api();
        let url_resource = use_resource(move || {
            let cx = cx_inner.clone();
            let asset_id = asset_id.clone();
            async move {
                let path = format!("/v1/file-assets/{asset_id}/url");
                fetch_json::<UrlResponse>(&cx, "GET", &path, None::<&()>)
                    .await
                    .map(|r| r.url)
            }
        });
        match &*url_resource.read_unchecked() {
            Some(Ok(url)) => rsx! {
                video { controls: true, src: "{url}", class: "lesson-video" }
            },
            Some(Err(_)) => rsx! { p { class: "form-error", "Video unavailable." } },
            None => rsx! { p { "Loading video…" } },
        }
    } else {
        rsx! { p { class: "muted", "No video uploaded yet." } }
    }
}

fn render_file_bundle(lesson: &LessonView, _cx: &ApiContext) -> Element {
    let lesson_id = lesson.id.clone();
    let cx_inner = api::use_api();
    let files = use_resource(move || {
        let cx = cx_inner.clone();
        let lesson_id = lesson_id.clone();
        async move {
            let path = format!("/v1/lessons/{lesson_id}/files");
            fetch_json::<Vec<LessonFile>>(&cx, "GET", &path, None::<&()>).await
        }
    });

    match &*files.read_unchecked() {
        Some(Ok(list)) if list.is_empty() => rsx! {
            p { class: "muted", "No files attached." }
        },
        Some(Ok(list)) => {
            let list_clone = list.clone();
            rsx! {
                ul { class: "lesson-file-list",
                    for f in list_clone {
                        {
                            let asset_id = f.asset_id.clone();
                            let filename = f.filename.clone();
                            let content_type = f.content_type.clone();
                            let size = f.size_bytes;
                            let cx_open = api::use_api();
                            rsx! {
                                li { key: "{asset_id}",
                                    FileCard {
                                        filename: filename,
                                        size_bytes: size,
                                        content_type: content_type,
                                        on_open: move |_| {
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                let cx = cx_open.clone();
                                                let asset_id = asset_id.clone();
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    let path = format!("/v1/file-assets/{asset_id}/url");
                                                    if let Ok(r) = fetch_json::<UrlResponse>(
                                                        &cx, "GET", &path, None::<&()>
                                                    ).await {
                                                        let win = web_sys::window().unwrap();
                                                        let _ = win.location().set_href(&r.url);
                                                    }
                                                });
                                            }
                                            #[cfg(not(target_arch = "wasm32"))]
                                            {
                                                let cx = cx_open.clone();
                                                let asset_id = asset_id.clone();
                                                spawn(async move {
                                                    let path = format!("/v1/file-assets/{asset_id}/url");
                                                    if let Ok(response) = fetch_json::<UrlResponse>(
                                                        &cx, "GET", &path, None::<&()>
                                                    ).await {
                                                        let _ = platform_bridge::external::open_external_url(
                                                            &response.url,
                                                            platform_bridge::external::ExternalPurpose::LearningContent,
                                                        );
                                                    }
                                                });
                                            }
                                        },
                                        on_delete: None,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(_)) => rsx! { p { class: "form-error", "Couldn't load files." } },
        None => rsx! { p { "Loading files…" } },
    }
}

#[allow(dead_code)]
fn _force_link() -> Element {
    rsx! { FileAssetImage { asset_id: "x".to_string(), alt: "x".to_string() } }
}

// ─── Phase 1c: lesson-attached assignments inline card ───────────────────────

use crate::api::{self as api_mod, AssignmentDto};

#[derive(Clone, Props, PartialEq)]
pub struct LessonAssignmentsCardProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub lesson_id: String,
}

pub fn LessonAssignmentsCard(props: LessonAssignmentsCardProps) -> Element {
    let api = props.api.clone();
    let lid = props.lesson_id.clone();
    let assignments = use_resource(move || {
        let api = api.clone();
        let lid = lid.clone();
        async move { api_mod::list_lesson_assignments(&api, &lid).await }
    });
    let course_slug = props.course_slug.clone();

    rsx! {
        section { class: "lesson-assignments",
            h3 { "Assignments for this lesson" }
            match &*assignments.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! { p { "None yet." } },
                Some(Ok(items)) => rsx! {
                    ul {
                        for a in items.iter() {
                            { render_lesson_card_row(&course_slug, a) }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading..." } },
            }
        }
    }
}

fn render_lesson_card_row(course_slug: &str, a: &AssignmentDto) -> Element {
    let id = a.id.clone();
    let title = a.title.clone();
    rsx! {
        li { key: "{id}",
            a { href: format!("/courses/{course_slug}/assignments/{id}"), "{title}" }
            if let Some(d) = a.due_at.as_ref() { span { class: "due", " · due {d}" } }
        }
    }
}
