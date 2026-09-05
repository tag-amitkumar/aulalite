// crates/features-courses/src/live_room_replay.rs
//! Recording playback view with time-synced chat side panel.

use crate::api::{self, fetch_json, ApiError};
use crate::attendance_panel::AttendancePanel;
use crate::live_room_chat::{ChatMessage, LiveRoomChat};
use crate::recording_chapters::RecordingChapters;
use design_system::{
    Button, ButtonVariant, EmptyState, EmptyStateVariant, HeadingLevel, Loading, PageHeader,
    PageHeaderVariant,
};
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone, PartialEq, Default)]
pub struct RecordingDto {
    pub session_id: String,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub duration_seconds: Option<i64>,
    pub started_at: Option<String>,
    pub playback_url: Option<String>,
    pub course_title: String,
    pub instructor_user_id: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq)]
struct RecordingChatMessageDto {
    pub id: String,
    pub sender_user_id: String,
    pub sender_display_name: String,
    pub body: String,
    pub video_offset_seconds: f64,
    pub deleted: bool,
}

#[derive(Deserialize, Clone, PartialEq, Default)]
struct RecordingChatWindow {
    pub messages: Vec<RecordingChatMessageDto>,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomReplayProps {
    pub session_id: String,
    pub course_title: String,
    pub instructor_name: Option<String>,
    pub is_teacher: bool,
}

pub fn LiveRoomReplay(props: LiveRoomReplayProps) -> Element {
    let cx = api::use_api();
    let session_id = props.session_id.clone();

    let recording = use_resource(move || {
        let cx = cx.clone();
        let session_id = session_id.clone();
        async move {
            fetch_json::<RecordingDto>(
                &cx,
                "GET",
                &format!("/v1/sessions/{session_id}/recording"),
                None::<&()>,
            )
            .await
        }
    });

    match &*recording.read_unchecked() {
        Some(Ok(rec)) => render_with_recording(&props, rec),
        Some(Err(ApiError::Status(404, _))) => rsx! {
            div { class: "live-room-replay-empty live-room-replay motion-page",
                EmptyState {
                    title: "Class has ended".to_string(),
                    description: "No recording is available for this session.".to_string(),
                    variant: EmptyStateVariant::Subtle,
                    cta: rsx! {
                        a { class: "ds-button ds-button--secondary", href: "/", "Back to dashboard" }
                    },
                }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "live-room-replay motion-page",
                EmptyState {
                    title: "Couldn't load recording".to_string(),
                    description: "Please refresh the page and try again.".to_string(),
                    variant: EmptyStateVariant::Subtle,
                    cta: None,
                }
            }
        },
        None => rsx! {
            div { class: "live-room-replay motion-page",
                Loading { message: "Loading recording\u{2026}".to_string() }
            }
        },
    }
}

fn render_with_recording(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    match rec.processing_status.as_str() {
        "available" => render_available(props, rec),
        "pending" | "remuxing" | "uploading" => rsx! {
            div { class: "live-room-replay-processing live-room-replay motion-page",
                PageHeader {
                    kicker: "Replay".to_string(),
                    title: props.course_title.clone(),
                    subtitle: "Recording is being processed (usually 5-15 min after class ends).".to_string(),
                    variant: PageHeaderVariant::Hero,
                    as_tag: HeadingLevel::H2,
                }
                p { class: "muted", "Refresh this page to check again." }
            }
        },
        "failed" => render_failed(props, rec),
        other => rsx! {
            div { class: "live-room-replay motion-page",
                EmptyState {
                    title: "Unknown recording status".to_string(),
                    description: format!("Status: {other}"),
                    variant: EmptyStateVariant::Subtle,
                    cta: None,
                }
            }
        },
    }
}

fn render_available(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    let session_id = props.session_id.clone();
    let video_seconds = use_signal(|| 0.0_f64);
    let chat_window = use_signal(Vec::<ChatMessage>::new);

    // 500ms polling effect: reads video.currentTime and drives video_seconds signal.
    #[cfg(target_arch = "wasm32")]
    let mut video_seconds_for_poll = video_seconds;
    #[cfg(not(target_arch = "wasm32"))]
    let video_seconds_for_poll = video_seconds;
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                loop {
                    if let Some(win) = web_sys::window() {
                        if let Some(doc) = win.document() {
                            if let Some(el) = doc.get_element_by_id("live-room-replay-video") {
                                use wasm_bindgen::JsCast;
                                if let Ok(media) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                                    let t = media.current_time();
                                    video_seconds_for_poll.set(t);
                                }
                            }
                        }
                    }
                    gloo_timers::future::TimeoutFuture::new(500).await;
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = video_seconds_for_poll;
        }
    });

    let cx = api::use_api();
    let session_id_for_effect = session_id.clone();
    let cx_for_effect = cx.clone();
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            let secs = *video_seconds.read();
            // Bucket at 30s boundaries to limit refetch frequency.
            let _bucket = (secs / 30.0) as i64;
            let to_secs = secs + 5.0;
            let cx = cx_for_effect.clone();
            let session_id = session_id_for_effect.clone();
            let mut chat_window_set = chat_window;
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(window) = fetch_json::<RecordingChatWindow>(
                    &cx, "GET",
                    &format!("/v1/sessions/{session_id}/recording/chat?from_seconds=0&to_seconds={to_secs}"),
                    None::<&()>,
                ).await {
                    let mapped: Vec<ChatMessage> = window.messages.into_iter().map(|m| ChatMessage {
                        id: m.id,
                        sender_display_name: m.sender_display_name,
                        body: m.body,
                        created_at: format!("@{:.1}s", m.video_offset_seconds),
                        deleted: m.deleted,
                    }).collect();
                    chat_window_set.set(mapped);
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (
                cx_for_effect.clone(),
                session_id_for_effect.clone(),
                video_seconds,
                chat_window,
            );
        }
    });

    let url = rec.playback_url.clone().unwrap_or_default();
    // The playback_url is a short-lived presigned GET URL (see
    // handlers::live_sessions::recording_inner -> presigned_get_url). Reuse it
    // directly as a same-window download via `<a download>`; no auth header is
    // needed because the signature is baked into the URL. The `download`
    // attribute hints a filename and asks the browser to save rather than
    // navigate. Hidden when there's no URL (shouldn't happen for "available").
    let has_url = !url.is_empty();
    let is_teacher = props.is_teacher;

    rsx! {
        div { class: "live-room-replay motion-page",
            PageHeader {
                kicker: "Replay".to_string(),
                title: props.course_title.clone(),
                subtitle: props
                    .instructor_name
                    .clone()
                    .map(|n| format!("Recorded session · {n}"))
                    .unwrap_or_else(|| "Recorded session".to_string()),
                variant: PageHeaderVariant::Hero,
                as_tag: HeadingLevel::H2,
            }
            div { class: "live-room-replay-grid",
                div { class: "replay-video-pane live-room-stage",
                    video {
                        id: "live-room-replay-video",
                        src: "{url}",
                        controls: true,
                        playsinline: true,
                        class: "replay-video",
                    }
                    if has_url {
                        div { class: "replay-video-actions",
                            a {
                                class: "ds-button ds-button--secondary ds-button--sm replay-download",
                                href: "{url}",
                                download: "recording.mp4",
                                rel: "noopener",
                                "Download recording"
                            }
                        }
                    }
                }
                div { class: "replay-chat-pane",
                    RecordingChapters {
                        session_id: session_id.clone(),
                        is_teacher,
                    }
                    LiveRoomChat {
                        messages: chat_window.read().clone(),
                        is_teacher: false,
                        on_send: move |_b: String| {},
                        on_delete: move |_id: String| {},
                    }
                }
            }
            // Staff-only attendance report. Self-hides for students because
            // the backend 403s non-staff viewers (handled inside the panel).
            AttendancePanel { session_id: session_id.clone() }
            crate::session_feedback::SessionFeedback {
                session_id: session_id.clone(),
                is_teacher,
            }
        }
    }
}

fn render_failed(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    let session_id = props.session_id.clone();
    let is_teacher = props.is_teacher;
    let cx = api::use_api();
    let on_retry = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let cx = cx.clone();
            let session_id = session_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _: Result<RecordingDto, _> = fetch_json(
                    &cx,
                    "POST",
                    &format!("/v1/sessions/{session_id}/recording/retry"),
                    Some(&serde_json::json!({})),
                )
                .await;
                if let Some(win) = web_sys::window() {
                    let _ = win.location().reload();
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (cx.clone(), session_id.clone());
        }
    };
    let err_text = rec.processing_error.clone().unwrap_or_default();
    rsx! {
        div { class: "live-room-replay-failed live-room-replay motion-page",
            if is_teacher {
                EmptyState {
                    title: "Recording failed to process".to_string(),
                    description: if err_text.is_empty() {
                        "Try processing again to recover the recording.".to_string()
                    } else {
                        err_text.clone()
                    },
                    variant: EmptyStateVariant::Subtle,
                    cta: rsx! {
                        Button {
                            label: "Retry processing".to_string(),
                            variant: ButtonVariant::Primary,
                            on_click: on_retry,
                        }
                    },
                }
            } else {
                EmptyState {
                    title: "Recording failed to process".to_string(),
                    description: "Please ask your instructor to retry the recording.".to_string(),
                    variant: EmptyStateVariant::Subtle,
                    cta: None,
                }
            }
        }
    }
}
