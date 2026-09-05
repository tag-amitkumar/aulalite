// crates/features-courses/src/lesson_video_editor.rs
use crate::api::{self, fetch_json};
use crate::file_picker::{validation, FilePicker};
use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct UrlResponse {
    url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonVideoEditorProps {
    pub lesson_id: String,
    pub current_video_asset_id: Option<String>,
    pub on_uploaded: EventHandler<String>, // new asset_id
}

#[component]
pub fn LessonVideoEditor(props: LessonVideoEditorProps) -> Element {
    let mut replacing = use_signal(|| false);
    let cx = api::use_api();

    let asset_present = props.current_video_asset_id.is_some() && !*replacing.read();

    rsx! {
        div { class: "lesson-video-editor",
            if asset_present {
                {
                    let asset_id = props.current_video_asset_id.clone().unwrap();
                    let cx_inner = cx.clone();
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
                            Button {
                                label: "Replace video".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: move |_| replacing.set(true),
                            }
                        },
                        Some(Err(_)) => rsx! {
                            div { class: "form-error", "Couldn't load video." }
                            Button {
                                label: "Replace video".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: move |_| replacing.set(true),
                            }
                        },
                        None => rsx! { div { "Loading…" } },
                    }
                }
            } else {
                FilePicker {
                    purpose: "video".to_string(),
                    linked_entity_type: "lesson".to_string(),
                    linked_entity_id: props.lesson_id.clone(),
                    allowed_types: validation::VIDEO_TYPES.iter().map(|s| s.to_string()).collect(),
                    max_size_bytes: validation::VIDEO_MAX,
                    on_uploaded: move |asset_id: String| {
                        replacing.set(false);
                        props.on_uploaded.call(asset_id);
                    },
                    button_label: "Upload video".to_string(),
                }
            }
        }
    }
}
