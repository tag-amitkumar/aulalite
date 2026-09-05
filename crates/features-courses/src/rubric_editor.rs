// crates/features-courses/src/rubric_editor.rs
//! Rubric builder embedded in the assignment editor. Teacher-only. Lets staff
//! attach a single titled rubric (a list of weighted criteria) to an
//! assignment via `POST /v1/assignments/:aid/rubric`, or remove it.
//!
//! A rubric only makes sense for numeric assignments (the per-criterion points
//! sum to the numeric grade at grade time), so the parent only mounts this once
//! the assignment exists and is in numeric grading mode.

use crate::api::{self, ApiContext, CriterionInput, RubricDto, UpsertRubricBody};
use design_system::{use_toast_sender, Button, ButtonVariant, Field, FormError, Input, ToastLevel};
use dioxus::prelude::*;

/// A single editable criterion row in the builder. `id` is `None` for rows the
/// teacher just added; existing criteria keep their server id for keying only
/// (the upsert always replaces the whole rubric).
#[derive(Clone, PartialEq)]
struct CriterionDraft {
    label: String,
    max_points: i32,
}

#[derive(Clone, Props, PartialEq)]
pub struct RubricEditorProps {
    pub api: ApiContext,
    pub assignment_id: String,
    /// The rubric currently attached to the assignment, if any (the parent
    /// fetches it). Drives the initial builder state.
    pub initial: Option<RubricDto>,
}

pub fn RubricEditor(props: RubricEditorProps) -> Element {
    let initial = props.initial.clone();
    let mut title = use_signal(|| {
        initial
            .as_ref()
            .map(|r| r.title.clone())
            .unwrap_or_else(|| "Rubric".to_string())
    });
    let mut criteria: Signal<Vec<CriterionDraft>> = use_signal(|| {
        initial
            .as_ref()
            .map(|r| {
                r.criteria
                    .iter()
                    .map(|c| CriterionDraft {
                        label: c.label.clone(),
                        max_points: c.max_points,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let mut has_rubric = use_signal(|| initial.is_some());
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut saving = use_signal(|| false);
    let mut toast = use_toast_sender();

    let api = props.api.clone();
    let assignment_id = props.assignment_id.clone();

    // Running total of the criteria max points — the achievable grade ceiling.
    let total_points: i32 = criteria.read().iter().map(|c| c.max_points).sum();

    let save_api = api.clone();
    let save_aid = assignment_id.clone();
    let on_save = move |_| {
        let api = save_api.clone();
        let aid = save_aid.clone();
        spawn(async move {
            let title_v = title.read().clone();
            let drafts = criteria.read().clone();
            if title_v.trim().is_empty() {
                error.set(Some("Give the rubric a title.".into()));
                return;
            }
            if drafts.is_empty() {
                error.set(Some("Add at least one criterion.".into()));
                return;
            }
            if drafts.iter().any(|c| c.label.trim().is_empty()) {
                error.set(Some("Every criterion needs a label.".into()));
                return;
            }
            if drafts.iter().any(|c| c.max_points <= 0) {
                error.set(Some(
                    "Every criterion needs a max-points value above 0.".into(),
                ));
                return;
            }
            saving.set(true);
            let criteria_body: Vec<CriterionInput> = drafts
                .iter()
                .map(|c| CriterionInput {
                    label: c.label.trim(),
                    max_points: c.max_points,
                })
                .collect();
            let body = UpsertRubricBody {
                title: title_v.trim(),
                criteria: criteria_body,
            };
            match api::upsert_assignment_rubric(&api, &aid, &body).await {
                Ok(_) => {
                    error.set(None);
                    has_rubric.set(true);
                    toast.push(
                        ToastLevel::Success,
                        "Rubric saved",
                        "Graders will score each criterion.",
                    );
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Rubric save failed", msg.clone());
                    error.set(Some(msg));
                }
            }
            saving.set(false);
        });
    };

    let del_api = api.clone();
    let del_aid = assignment_id.clone();
    let on_delete = move |_| {
        let api = del_api.clone();
        let aid = del_aid.clone();
        spawn(async move {
            saving.set(true);
            match api::delete_assignment_rubric(&api, &aid).await {
                Ok(_) => {
                    has_rubric.set(false);
                    criteria.set(Vec::new());
                    error.set(None);
                    toast.push(
                        ToastLevel::Success,
                        "Rubric removed",
                        "This assignment no longer uses a rubric.",
                    );
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Remove failed", msg.clone());
                    error.set(Some(msg));
                }
            }
            saving.set(false);
        });
    };

    rsx! {
        fieldset { class: "assignment-editor__fieldset rubric-editor",
            legend { "Rubric (optional)" }
            p { class: "muted rubric-editor__hint",
                "Add weighted criteria to grade this numeric assignment per-criterion. The criterion scores sum to the grade."
            }

            Field { label: "Rubric title".to_string(),
                Input {
                    value: title.read().clone(),
                    input_type: "text".to_string(),
                    on_input: move |v| title.set(v),
                }
            }

            if criteria.read().is_empty() {
                p { class: "muted rubric-editor__empty", "No criteria yet. Add your first criterion below." }
            } else {
                ul { class: "rubric-editor__criteria",
                    for (idx, c) in criteria.read().iter().enumerate() {
                        li { key: "{idx}", class: "rubric-editor__criterion",
                            Input {
                                value: c.label.clone(),
                                input_type: "text".to_string(),
                                on_input: move |v: String| {
                                    let mut list = criteria.read().clone();
                                    if let Some(item) = list.get_mut(idx) { item.label = v; }
                                    criteria.set(list);
                                },
                            }
                            Input {
                                value: format!("{}", c.max_points),
                                input_type: "number".to_string(),
                                on_input: move |v: String| {
                                    if let Ok(n) = v.parse::<i32>() {
                                        let mut list = criteria.read().clone();
                                        if let Some(item) = list.get_mut(idx) { item.max_points = n.max(0); }
                                        criteria.set(list);
                                    }
                                },
                            }
                            Button {
                                label: "Remove".to_string(),
                                variant: ButtonVariant::Ghost,
                                on_click: move |_| {
                                    let mut list = criteria.read().clone();
                                    if idx < list.len() { list.remove(idx); }
                                    criteria.set(list);
                                },
                            }
                        }
                    }
                }
                p { class: "rubric-editor__total", "Total points: {total_points}" }
            }

            div { class: "rubric-editor__actions",
                Button {
                    label: "Add criterion".to_string(),
                    variant: ButtonVariant::Secondary,
                    on_click: move |_| {
                        let mut list = criteria.read().clone();
                        list.push(CriterionDraft { label: String::new(), max_points: 10 });
                        criteria.set(list);
                    },
                }
                Button {
                    label: if *saving.read() { "Saving…".to_string() } else { "Save rubric".to_string() },
                    variant: ButtonVariant::Primary,
                    disabled: *saving.read(),
                    on_click: on_save,
                }
                if *has_rubric.read() {
                    Button {
                        label: "Remove rubric".to_string(),
                        variant: ButtonVariant::Danger,
                        disabled: *saving.read(),
                        on_click: on_delete,
                    }
                }
            }

            FormError { message: error.read().clone() }
        }
    }
}
