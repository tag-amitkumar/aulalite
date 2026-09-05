// crates/features-courses/src/course_cover_editor.rs
use crate::file_asset_image::FileAssetImage;
use crate::file_picker::{validation, FilePicker};
use design_system::Card;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseCoverEditorProps {
    pub course_id: String,
    pub current_cover_asset_id: Option<String>,
    pub on_changed: EventHandler<String>, // new asset_id
}

#[component]
pub fn CourseCoverEditor(props: CourseCoverEditorProps) -> Element {
    rsx! {
        Card {
            h3 { "Cover image" }
            if let Some(asset_id) = &props.current_cover_asset_id {
                FileAssetImage {
                    asset_id: asset_id.clone(),
                    alt: "Course cover".to_string(),
                    class: Some("cover-preview".to_string()),
                }
            } else {
                // No upload yet: show the branded generated cover that learners
                // currently see (instead of bare "No cover image yet." text), so
                // the editor previews the real fallback.
                design_system::CourseCoverArt {
                    seed: props.course_id.clone(),
                    title: "Cover".to_string(),
                    class: "cover-preview course-gen-cover".to_string(),
                }
                p { class: "muted", "Using a generated cover. Upload an image to replace it." }
            }
            FilePicker {
                purpose: "cover".to_string(),
                linked_entity_type: "course".to_string(),
                linked_entity_id: props.course_id.clone(),
                allowed_types: validation::COVER_TYPES.iter().map(|s| s.to_string()).collect(),
                max_size_bytes: validation::COVER_MAX,
                on_uploaded: move |asset_id: String| props.on_changed.call(asset_id),
                button_label: "Upload cover image".to_string(),
            }
        }
    }
}
