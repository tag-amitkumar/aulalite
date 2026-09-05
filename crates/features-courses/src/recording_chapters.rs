// crates/features-courses/src/recording_chapters.rs
//! Chapters panel for the recording replay view.
//!
//! Lists a recording's chapters (named timestamp bookmarks). Clicking a chapter
//! seeks the replay `<video id="live-room-replay-video">` by setting its
//! `currentTime`. Course staff additionally get an inline "add chapter at
//! current time" affordance and per-row delete; the chapter endpoints 403
//! non-staff callers, so the add/delete controls are hidden for students.
//!
//! Keyed by `session_id` to match the `/v1/sessions/:id/recording*` URL family;
//! the backend resolves the recording id from the session.

use crate::api::{self, fetch_json};
use design_system::{Button, ButtonSize, ButtonVariant};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, PartialEq)]
pub struct ChapterDto {
    pub id: String,
    pub label: String,
    pub position_seconds: i64,
}

#[derive(Serialize)]
struct CreateChapterBody {
    label: String,
    position_seconds: i64,
}

/// Format an integer second offset as `H:MM:SS` (or `M:SS` under an hour).
fn fmt_timestamp(total: i64) -> String {
    let total = total.max(0);
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Seek the replay `<video>` to `seconds` and resume playback. No-op off wasm.
fn seek_video(seconds: i64) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(win) = web_sys::window() {
            if let Some(doc) = win.document() {
                if let Some(el) = doc.get_element_by_id("live-room-replay-video") {
                    if let Ok(media) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                        media.set_current_time(seconds.max(0) as f64);
                        let _ = media.play();
                    }
                }
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = seconds;
    }
}

/// Read the replay `<video>`'s current playback position, rounded to whole
/// seconds. Returns 0 off wasm or when the element isn't present.
fn current_video_seconds() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(win) = web_sys::window() {
            if let Some(doc) = win.document() {
                if let Some(el) = doc.get_element_by_id("live-room-replay-video") {
                    if let Ok(media) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                        return media.current_time().floor() as i64;
                    }
                }
            }
        }
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct RecordingChaptersProps {
    pub session_id: String,
    pub is_teacher: bool,
}

#[component]
pub fn RecordingChapters(props: RecordingChaptersProps) -> Element {
    let cx = api::use_api();
    let session_id = props.session_id.clone();
    let is_teacher = props.is_teacher;

    // Bumped to force a refetch after add/delete.
    let reload = use_signal(|| 0_u32);
    let new_label = use_signal(String::new);

    let chapters = {
        let cx = cx.clone();
        let session_id = session_id.clone();
        use_resource(move || {
            let cx = cx.clone();
            let session_id = session_id.clone();
            let _ = reload.read();
            async move {
                fetch_json::<Vec<ChapterDto>>(
                    &cx,
                    "GET",
                    &format!("/v1/sessions/{session_id}/recording/chapters"),
                    None::<&()>,
                )
                .await
            }
        })
    };

    let add_chapter = {
        let cx = cx.clone();
        let session_id = session_id.clone();
        let new_label = new_label;
        move |_| {
            let label = new_label.read().trim().to_string();
            if label.is_empty() {
                return;
            }
            let position_seconds = current_video_seconds();
            let cx = cx.clone();
            let session_id = session_id.clone();
            #[cfg(target_arch = "wasm32")]
            {
                let mut new_label = new_label;
                let mut reload = reload;
                wasm_bindgen_futures::spawn_local(async move {
                    let body = CreateChapterBody {
                        label,
                        position_seconds,
                    };
                    let res: Result<ChapterDto, _> = fetch_json(
                        &cx,
                        "POST",
                        &format!("/v1/sessions/{session_id}/recording/chapters"),
                        Some(&body),
                    )
                    .await;
                    if res.is_ok() {
                        new_label.set(String::new());
                        reload.with_mut(|n| *n += 1);
                    }
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut new_label = new_label;
                let _ = (cx, session_id, label, position_seconds, reload);
                new_label.set(String::new());
            }
        }
    };

    let chapter_rows = match &*chapters.read_unchecked() {
        Some(Ok(list)) if !list.is_empty() => {
            let items = list.clone();
            rsx! {
                ul { class: "recording-chapters-list",
                    for ch in items {
                        ChapterRow {
                            key: "{ch.id}",
                            session_id: session_id.clone(),
                            chapter: ch.clone(),
                            is_teacher,
                            reload,
                        }
                    }
                }
            }
        }
        Some(Ok(_)) => rsx! {
            p { class: "muted recording-chapters-empty", "No chapters yet." }
        },
        // A 403 (student hitting a staff-only error) or any other failure simply
        // renders nothing notable; the list endpoint is read-gated, so a genuine
        // student viewer gets an empty/quiet panel rather than a scary error.
        Some(Err(_)) => rsx! {
            p { class: "muted recording-chapters-empty", "No chapters available." }
        },
        None => rsx! {
            p { class: "muted recording-chapters-empty", "Loading chapters\u{2026}" }
        },
    };

    rsx! {
        section { class: "recording-chapters",
            h3 { class: "recording-chapters-title", "Chapters" }
            {chapter_rows}
            if is_teacher {
                div { class: "recording-chapters-add",
                    input {
                        r#type: "text",
                        class: "ds-input recording-chapters-add-input",
                        placeholder: "Chapter label",
                        value: "{new_label}",
                        maxlength: "120",
                        oninput: move |e| {
                            let mut new_label = new_label;
                            new_label.set(e.value());
                        },
                    }
                    Button {
                        label: "Add at current time".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_click: add_chapter,
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ChapterRowProps {
    session_id: String,
    chapter: ChapterDto,
    is_teacher: bool,
    /// Parent's refetch counter; bumped after a successful delete.
    reload: Signal<u32>,
}

#[component]
fn ChapterRow(props: ChapterRowProps) -> Element {
    let cx = api::use_api();
    let pos = props.chapter.position_seconds;
    let label = props.chapter.label.clone();
    let is_teacher = props.is_teacher;

    let on_delete = {
        let cx = cx.clone();
        let session_id = props.session_id.clone();
        let chapter_id = props.chapter.id.clone();
        let reload = props.reload;
        move |_| {
            let cx = cx.clone();
            let session_id = session_id.clone();
            let chapter_id = chapter_id.clone();
            #[cfg(target_arch = "wasm32")]
            {
                let mut reload = reload;
                wasm_bindgen_futures::spawn_local(async move {
                    let res: Result<(), _> = fetch_json(
                        &cx,
                        "DELETE",
                        &format!("/v1/sessions/{session_id}/recording/chapters/{chapter_id}"),
                        None::<&()>,
                    )
                    .await;
                    if res.is_ok() {
                        reload.with_mut(|n| *n += 1);
                    }
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut reload = reload;
                let _ = (cx, session_id, chapter_id);
                reload.with_mut(|n| *n += 1);
            }
        }
    };

    rsx! {
        li { class: "recording-chapter-row",
            button {
                r#type: "button",
                class: "recording-chapter-seek",
                onclick: move |_| seek_video(pos),
                span { class: "recording-chapter-time", "{fmt_timestamp(pos)}" }
                span { class: "recording-chapter-label", "{label}" }
            }
            if is_teacher {
                Button {
                    label: "Remove".to_string(),
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    on_click: on_delete,
                }
            }
        }
    }
}
