// crates/features-courses/src/lesson_editor.rs
use design_system::{
    Button, ButtonVariant, Checkbox, DateTimePicker, Input, MarkdownEditor, Select, SelectOption,
};
use dioxus::prelude::*;

use crate::api::{self, fetch_json};
use crate::lesson_files_editor::LessonFilesEditor;
use crate::lesson_video_editor::LessonVideoEditor;

#[derive(Clone, PartialEq)]
pub struct UnscheduledSession {
    pub id: String,
    pub label: String,
}

/// A sibling lesson the editor can offer as a prerequisite (id + title).
#[derive(Clone, PartialEq)]
pub struct SiblingLesson {
    pub id: String,
    pub title: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonEditorProps {
    pub lesson_id: String,
    pub r#type: String,
    pub title: String,
    pub body_md: String,
    pub linked_session_id: Option<String>,
    pub linked_video_asset_id: Option<String>,
    pub available_sessions: Vec<UnscheduledSession>,
    pub on_save: EventHandler<SaveLessonRequest>,
    pub on_video_uploaded: EventHandler<String>, // (asset_id)

    // ── Prerequisites + drip (staff). All optional so existing call sites keep
    // compiling; the picker + release-date controls only render when both
    // `course_id` and `module_id` are supplied. ──
    #[props(default)]
    pub course_id: Option<String>,
    #[props(default)]
    pub module_id: Option<String>,
    /// Other lessons in the course, offered as prerequisite candidates.
    #[props(default)]
    pub sibling_lessons: Vec<SiblingLesson>,
    /// Current `release_at` in RFC3339 (from the lesson GET), if any.
    #[props(default)]
    pub current_release_at: Option<String>,
}

#[derive(serde::Deserialize, Clone, PartialEq)]
struct PrerequisiteResp {
    required_lesson_id: String,
    #[allow(dead_code)]
    required_lesson_title: String,
}

#[derive(serde::Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct SetPrereqBody {
    required_lesson_ids: Vec<String>,
}

#[derive(serde::Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct ReleaseAtBody {
    // double-option on the wire: present-and-null clears, present-and-set gates.
    release_at: Option<String>,
}

/// `datetime-local` (`YYYY-MM-DDTHH:MM`) → RFC3339 UTC (`...:00Z`). Empty ⇒ None.
fn local_to_rfc3339(local: &str) -> Option<String> {
    let t = local.trim();
    if t.is_empty() {
        None
    } else {
        Some(format!("{t}:00Z"))
    }
}

/// RFC3339 → `datetime-local` value (`YYYY-MM-DDTHH:MM`) for the input.
fn rfc3339_to_local(rfc: &str) -> String {
    // Keep the date + HH:MM; tolerate both `T` separators and trailing zone.
    let bytes = rfc.as_bytes();
    if bytes.len() >= 16 {
        rfc[..16].to_string()
    } else {
        rfc.to_string()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct SaveLessonRequest {
    pub lesson_id: String,
    pub title: String,
    pub body_md: Option<String>,
    pub linked_session_id: Option<String>,
}

#[component]
pub fn LessonEditor(props: LessonEditorProps) -> Element {
    let mut title = use_signal(|| props.title.clone());
    let mut body = use_signal(|| props.body_md.clone());
    let mut linked = use_signal(|| props.linked_session_id.clone());
    let r#type = props.r#type.clone();

    // Drip/prereq controls are only active when the course context is wired.
    let gating_enabled = props.course_id.is_some() && props.module_id.is_some();

    // Release-date field (datetime-local value).
    let mut release_local = use_signal(|| {
        props
            .current_release_at
            .as_deref()
            .map(rfc3339_to_local)
            .unwrap_or_default()
    });

    // Selected prerequisite lesson ids. Seeded from the server when gating is on.
    let mut selected_prereqs = use_signal(Vec::<String>::new);
    let gate_status = use_signal(String::new);

    // Load the lesson's existing prerequisites once.
    {
        let cx = api::use_api();
        let course_id = props.course_id.clone();
        let lesson_id = props.lesson_id.clone();
        let _prereqs = use_resource(move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let lesson_id = lesson_id.clone();
            async move {
                let Some(course_id) = course_id else {
                    return;
                };
                let path = format!("/v1/courses/{course_id}/lessons/{lesson_id}/prerequisites");
                if let Ok(list) =
                    fetch_json::<Vec<PrerequisiteResp>>(&cx, "GET", &path, None::<&()>).await
                {
                    selected_prereqs.set(list.into_iter().map(|p| p.required_lesson_id).collect());
                }
            }
        });
    }

    let do_save = {
        let on_save = props.on_save;
        let lesson_id = props.lesson_id.clone();
        let r#type = r#type.clone();
        let cx = api::use_api();
        let course_id = props.course_id.clone();
        let module_id = props.module_id.clone();
        move |_| {
            let body_md = if r#type == "rich_text" {
                Some(body.read().clone())
            } else {
                None
            };
            let linked_session_id = if r#type == "live_session" {
                linked.read().clone()
            } else {
                None
            };
            on_save.call(SaveLessonRequest {
                lesson_id: lesson_id.clone(),
                title: title.read().clone(),
                body_md,
                linked_session_id,
            });

            // Persist prerequisites + release_at directly (separate endpoints).
            if let (Some(course_id), Some(module_id)) = (course_id.clone(), module_id.clone()) {
                let cx = cx.clone();
                let lesson_id = lesson_id.clone();
                let prereqs = selected_prereqs.read().clone();
                let release = local_to_rfc3339(release_local.read().as_str());
                let mut gate_status = gate_status;
                #[cfg(target_arch = "wasm32")]
                wasm_bindgen_futures::spawn_local(async move {
                    let prereq_path =
                        format!("/v1/courses/{course_id}/lessons/{lesson_id}/prerequisites");
                    let ok1 = fetch_json::<Vec<PrerequisiteResp>>(
                        &cx,
                        "PUT",
                        &prereq_path,
                        Some(&SetPrereqBody {
                            required_lesson_ids: prereqs,
                        }),
                    )
                    .await
                    .is_ok();
                    let patch_path =
                        format!("/v1/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}");
                    let ok2 = fetch_json::<serde_json::Value>(
                        &cx,
                        "PATCH",
                        &patch_path,
                        Some(&ReleaseAtBody {
                            release_at: release,
                        }),
                    )
                    .await
                    .is_ok();
                    gate_status.set(if ok1 && ok2 {
                        "Prerequisites & release date saved.".to_string()
                    } else {
                        "Saved the lesson, but gating settings failed to save.".to_string()
                    });
                });
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let _ = (course_id, module_id, cx, lesson_id, prereqs, release);
                    gate_status.set(String::new());
                }
            }
        }
    };

    rsx! {
        div { class: "lesson-editor",
            div { class: "field",
                label { "Title" }
                Input {
                    value: title.read().clone(),
                    placeholder: "Lesson title".to_string(),
                    input_type: "text".to_string(),
                    disabled: false,
                    on_input: move |v| title.set(v),
                }
            }
            match r#type.as_str() {
                "rich_text" => rsx! {
                    div { class: "field",
                        label { "Content (markdown)" }
                        MarkdownEditor {
                            value: body.read().clone(),
                            on_change: move |v| body.set(v),
                            disabled: false,
                        }
                    }
                },
                "live_session" => rsx! {
                    div { class: "field",
                        label { "Linked live session" }
                        Select {
                            value: linked.read().clone().unwrap_or_default(),
                            options: {
                                let mut opts = vec![SelectOption { value: String::new(), label: "— pick a session —".to_string() }];
                                for s in &props.available_sessions {
                                    opts.push(SelectOption { value: s.id.clone(), label: s.label.clone() });
                                }
                                opts
                            },
                            on_change: move |v: String| linked.set(if v.is_empty() { None } else { Some(v) }),
                        }
                    }
                },
                "video" => rsx! {
                    LessonVideoEditor {
                        lesson_id: props.lesson_id.clone(),
                        current_video_asset_id: props.linked_video_asset_id.clone(),
                        on_uploaded: props.on_video_uploaded,
                    }
                },
                "file_bundle" => rsx! {
                    LessonFilesEditor {
                        lesson_id: props.lesson_id.clone(),
                    }
                },
                _ => rsx! { p { "Unknown lesson type: {r#type}" } },
            }

            // ── Access controls (staff): drip release date + prerequisites ──
            if gating_enabled {
                section { class: "lesson-gating",
                    h4 { "Access controls" }
                    div { class: "field",
                        label { "Release date (drip)" }
                        DateTimePicker {
                            value: release_local.read().clone(),
                            on_change: move |v: String| release_local.set(v),
                        }
                        p { class: "field-hint muted",
                            "Students can't open this lesson until this date. Leave blank to release immediately."
                        }
                    }
                    div { class: "field",
                        label { "Prerequisite lessons" }
                        if props.sibling_lessons.is_empty() {
                            p { class: "muted", "No other lessons to require yet." }
                        } else {
                            ul { class: "prereq-picker",
                                for sib in props.sibling_lessons.iter() {
                                    {
                                        let sid = sib.id.clone();
                                        let key_id = sib.id.clone();
                                        let title = sib.title.clone();
                                        let checked = selected_prereqs.read().contains(&sid);
                                        rsx! {
                                            li { key: "{key_id}", class: "prereq-picker-row",
                                                Checkbox {
                                                    checked,
                                                    label: title,
                                                    on_change: move |v: bool| {
                                                        let mut cur = selected_prereqs.read().clone();
                                                        if v {
                                                            if !cur.contains(&sid) { cur.push(sid.clone()); }
                                                        } else {
                                                            cur.retain(|x| x != &sid);
                                                        }
                                                        selected_prereqs.set(cur);
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            p { class: "field-hint muted",
                                "Students must complete every checked lesson before this one unlocks."
                            }
                        }
                    }
                    if !gate_status.read().is_empty() {
                        p { class: "form-status", "{gate_status}" }
                    }
                }
            }

            div { class: "actions",
                Button {
                    label: "Save".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: do_save,
                }
            }
        }
    }
}
