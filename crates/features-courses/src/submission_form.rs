// crates/features-courses/src/submission_form.rs
//! Student-facing submit form. Renders text + file inputs based on the
//! assignment's accepted types.

use crate::api::{self, ApiContext, AssignmentDto, PatchSubmissionBody, SubmissionDto};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionFormProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
}

pub fn SubmissionForm(props: SubmissionFormProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment.id.clone();
    let mut submission: Signal<Option<SubmissionDto>> = use_signal(|| None);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let api_for_load = api.clone();
    let aid_for_load = aid.clone();
    use_future(move || {
        let api = api_for_load.clone();
        let aid = aid_for_load.clone();
        async move {
            match api::create_or_get_submission(&api, &aid).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        }
    });

    let mut text_answer = use_signal(String::new);

    use_effect(move || {
        if let Some(s) = submission.read().as_ref() {
            text_answer.set(s.text_answer.clone().unwrap_or_default());
        }
    });

    let lock_on_submit = props.assignment.lock_on_submit;
    let editable = move |s: &SubmissionDto| -> bool {
        match s.status.as_str() {
            "draft" | "returned" => true,
            "submitted" => !lock_on_submit,
            _ => false,
        }
    };

    let api_for_save = api.clone();
    let on_save_draft = move |_| {
        let api = api_for_save.clone();
        let sid = submission
            .read()
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let body_text = text_answer.read().clone();
        spawn(async move {
            let body = PatchSubmissionBody {
                text_answer: Some(body_text.as_str()),
                attachment_asset_ids: None,
            };
            match api::patch_submission(&api, &sid, &body).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    let api_for_submit = api.clone();
    let on_submit = move |_| {
        let api = api_for_submit.clone();
        let sid = submission
            .read()
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        spawn(async move {
            match api::submit_submission(&api, &sid).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    let accepts_text = props.assignment.accepts_text;
    let accepts_files = props.assignment.accepts_files;

    rsx! {
        div { class: "submission-form",
            match submission.read().as_ref() {
                Some(s) => {
                    let edit = editable(s);
                    rsx! {
                        p { class: "status", "Status: {s.status}" }
                        if let Some(released) = s.released_at.as_ref() {
                            if let Some(g) = s.numeric_grade {
                                p { class: "grade", "Grade: {g} (released {released})" }
                            }
                            if let Some(fb) = s.student_visible_feedback.as_ref() {
                                p { class: "feedback", "Feedback: {fb}" }
                            }
                        }
                        if accepts_text {
                            label { "Your answer" }
                            textarea {
                                rows: 8,
                                disabled: !edit,
                                value: "{text_answer}",
                                oninput: move |e| text_answer.set(e.value()),
                            }
                        }
                        if accepts_files && edit {
                            crate::file_picker::FilePicker {
                                purpose: "attachment".to_string(),
                                linked_entity_type: "submission_attachment".to_string(),
                                linked_entity_id: s.id.clone(),
                                allowed_types: vec![
                                    "application/pdf".into(),
                                    "image/png".into(),
                                    "image/jpeg".into(),
                                    "text/plain".into(),
                                ],
                                max_size_bytes: 50 * 1024 * 1024,
                                button_label: "Attach a file".to_string(),
                                on_uploaded: {
                                    let api = api.clone();
                                    let sid = s.id.clone();
                                    let existing = s.attachment_asset_ids.clone();
                                    move |asset_id: String| {
                                        let api = api.clone();
                                        let sid = sid.clone();
                                        let mut existing = existing.clone();
                                        existing.push(asset_id);
                                        spawn(async move {
                                            let ids: Vec<&str> =
                                                existing.iter().map(|s| s.as_str()).collect();
                                            let body = api::PatchSubmissionBody {
                                                text_answer: None,
                                                attachment_asset_ids: Some(ids),
                                            };
                                            match api::patch_submission(&api, &sid, &body).await {
                                                Ok(s) => submission.set(Some(s)),
                                                Err(e) => error.set(Some(format!("{e}"))),
                                            }
                                        });
                                    }
                                },
                            }
                            ul { class: "submission-form__attachments",
                                for id in s.attachment_asset_ids.iter() {
                                    li { key: "{id}",
                                        crate::submission_view::FileAssetDownloadLink { asset_id: id.clone() }
                                    }
                                }
                            }
                        }
                        if edit {
                            button { onclick: on_save_draft, "Save draft" }
                            button { class: "btn-primary", onclick: on_submit, "Submit" }
                        }
                    }
                },
                None => rsx! { p { "Loading submission..." } },
            }
            if let Some(e) = error.read().as_ref() { p { class: "error", "{e}" } }
        }
    }
}
