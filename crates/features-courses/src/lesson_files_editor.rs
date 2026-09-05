// crates/features-courses/src/lesson_files_editor.rs
use crate::api::ApiError;
use crate::api::{self, fetch_json};
use crate::file_picker::{validation, FilePicker};
use design_system::{Card, FileCard};
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct LessonFile {
    asset_id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonFilesEditorProps {
    pub lesson_id: String,
}

#[component]
pub fn LessonFilesEditor(props: LessonFilesEditorProps) -> Element {
    let cx = api::use_api();
    let lesson_id = props.lesson_id.clone();

    let mut reload = use_signal(|| 0u32);

    let lesson_id_for_resource = lesson_id.clone();
    let cx_for_resource = cx.clone();
    let files = use_resource(move || {
        let _ = reload.read();
        let cx = cx_for_resource.clone();
        let lid = lesson_id_for_resource.clone();
        async move {
            let path = format!("/v1/lessons/{lid}/files");
            fetch_json::<Vec<LessonFile>>(&cx, "GET", &path, None::<&()>).await
        }
    });

    let lesson_id_for_picker = lesson_id.clone();
    rsx! {
        Card {
            h3 { "Attached files" }
            match &*files.read_unchecked() {
                Some(Ok(list)) if list.is_empty() => rsx! {
                    p { class: "muted", "No files attached yet." }
                },
                Some(Ok(list)) => {
                    let cx_for_delete = cx.clone();
                    rsx! {
                        ul { class: "lesson-files-list",
                            for f in list.clone() {
                                {
                                    let cx_d = cx_for_delete.clone();
                                    let asset_id = f.asset_id.clone();
                                    let reload_inner = reload;
                                    rsx! {
                                        li { key: "{asset_id}",
                                            FileCard {
                                                filename: f.filename,
                                                size_bytes: f.size_bytes,
                                                content_type: f.content_type,
                                                on_open: move |_| {},
                                                on_delete: Some(EventHandler::new(move |_| {
                                                    let cx = cx_d.clone();
                                                    let asset_id = asset_id.clone();
                                                    let mut reload_set = reload_inner;
                                                    spawn(async move {
                                                        let path = format!("/v1/file-assets/{asset_id}");
                                                        let _: Result<serde_json::Value, ApiError> =
                                                            fetch_json(&cx, "DELETE", &path, None::<&()>).await;
                                                        let next = *reload_set.read() + 1;
                                                        reload_set.set(next);
                                                    });
                                                })),
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
            FilePicker {
                purpose: "attachment".to_string(),
                linked_entity_type: "lesson".to_string(),
                linked_entity_id: lesson_id_for_picker,
                allowed_types: validation::ATTACHMENT_TYPES.iter().map(|s| s.to_string()).collect(),
                max_size_bytes: validation::ATTACHMENT_MAX,
                on_uploaded: move |_asset_id: String| {
                    let next = *reload.read() + 1;
                    reload.set(next);
                },
                button_label: "+ Add files".to_string(),
            }
        }
    }
}
