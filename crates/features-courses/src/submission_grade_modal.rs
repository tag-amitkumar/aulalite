// crates/features-courses/src/submission_grade_modal.rs
//! Inline grade-entry modal. Numeric input + optional letter, OR pass/fail
//! radio (per assignment.grading_mode). When the (numeric) assignment carries a
//! rubric, the numeric field is replaced by per-criterion score inputs whose
//! sum is the grade. Buttons: Save, Save & Release, Save & Return.

use crate::api::{
    self, ApiContext, AssignmentDto, CriterionScoreInput, GradeBody, RubricDto, SubmissionDto,
};
use design_system::{Button, ButtonVariant, Field, FormError, Input, Modal, ModalSize, Radio};
use dioxus::prelude::*;
use std::collections::HashMap;

#[derive(Props, Clone, PartialEq)]
pub struct SubmissionGradeModalProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
    pub submission: SubmissionDto,
    pub on_close: EventHandler<()>,
}

pub fn SubmissionGradeModal(props: SubmissionGradeModalProps) -> Element {
    let mut numeric: Signal<Option<f64>> = use_signal(|| props.submission.numeric_grade);
    let mut letter: Signal<String> =
        use_signal(|| props.submission.letter_grade.clone().unwrap_or_default());
    let mut passed: Signal<Option<bool>> = use_signal(|| props.submission.passed);
    let mut feedback: Signal<String> = use_signal(|| {
        props
            .submission
            .student_visible_feedback
            .clone()
            .unwrap_or_default()
    });
    let mut error: Signal<Option<String>> = use_signal(|| None);

    // Per-criterion points keyed by criterion id, populated as the grader types.
    let criterion_points: Signal<HashMap<String, f64>> = use_signal(HashMap::new);

    let api = props.api.clone();
    let sid = props.submission.id.clone();
    let mode = props.assignment.grading_mode.clone();
    let release_mode = props.assignment.release_mode.clone();
    let max_points = props.assignment.max_points.unwrap_or(0);
    let attempt_number = props.submission.attempt_number;
    let late_penalty_percent = props.assignment.late_penalty_percent;
    let submission_is_late = props.submission.is_late;
    // A late numeric submission on an assignment with a configured penalty will
    // have the penalty applied automatically when the grade is saved/released.
    let penalty_will_apply = mode == "numeric" && submission_is_late && late_penalty_percent > 0;

    // Fetch the assignment's rubric (numeric mode only). `None` => no rubric =>
    // the legacy numeric input is shown. Staff-only endpoint; the grader is
    // already course staff here.
    let rubric_api = api.clone();
    let rubric_aid = props.assignment.id.clone();
    let mode_for_rubric = mode.clone();
    let rubric_res = use_resource(move || {
        let api = rubric_api.clone();
        let aid = rubric_aid.clone();
        let mode = mode_for_rubric.clone();
        async move {
            if mode == "numeric" {
                api::get_assignment_rubric(&api, &aid).await
            } else {
                Ok(None)
            }
        }
    });
    let rubric: Option<RubricDto> = match &*rubric_res.read_unchecked() {
        Some(Ok(r)) => r.clone(),
        _ => None,
    };
    let use_rubric = rubric.is_some();

    // Live rubric total (sum of entered per-criterion points).
    let rubric_total: f64 = {
        let pts = criterion_points.read();
        rubric
            .as_ref()
            .map(|r| {
                r.criteria
                    .iter()
                    .map(|c| pts.get(&c.id).copied().unwrap_or(0.0))
                    .sum()
            })
            .unwrap_or(0.0)
    };

    let on_close_save = props.on_close;
    let api_save = api.clone();
    let sid_save = sid.clone();
    let mode_save = mode.clone();
    let rubric_save = rubric.clone();
    let on_save = move |_| {
        let api = api_save.clone();
        let sid = sid_save.clone();
        let mode = mode_save.clone();
        let rubric = rubric_save.clone();
        let on_close = on_close_save;
        spawn(async move {
            // Rubric path (numeric + rubric present): send per-criterion scores;
            // the backend sums them into the numeric grade.
            let criteria: Vec<CriterionScoreInput> = if mode == "numeric" {
                if let Some(r) = rubric.as_ref() {
                    let pts = criterion_points.read();
                    r.criteria
                        .iter()
                        .map(|c| CriterionScoreInput {
                            criterion_id: c.id.clone(),
                            points: pts.get(&c.id).copied().unwrap_or(0.0),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Legacy numeric value only when there is no rubric.
            let n = if mode == "numeric" && criteria.is_empty() {
                *numeric.read()
            } else {
                None
            };
            let l = if mode == "numeric" {
                let s = letter.read().clone();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            };
            let p = if mode == "pass_fail" {
                *passed.read()
            } else {
                None
            };
            let fb = feedback.read().clone();
            let body = GradeBody {
                numeric_grade: n,
                letter_grade: l.as_deref(),
                passed: p,
                student_visible_feedback: if fb.is_empty() {
                    None
                } else {
                    Some(fb.as_str())
                },
                teacher_only_notes: None,
                criteria,
            };
            match api::grade_submission(&api, &sid, &body).await {
                Ok(_) => on_close.call(()),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    let on_close_release = props.on_close;
    let api_release = api.clone();
    let sid_release = sid.clone();
    let on_save_release = move |_| {
        let api = api_release.clone();
        let sid = sid_release.clone();
        let on_close = on_close_release;
        spawn(async move {
            if let Err(e) = api::release_submission(&api, &sid).await {
                error.set(Some(format!("{e}")));
            } else {
                on_close.call(());
            }
        });
    };

    let on_close_return = props.on_close;
    let api_return = api.clone();
    let sid_return = sid.clone();
    let on_return = move |_| {
        let api = api_return.clone();
        let sid = sid_return.clone();
        let on_close = on_close_return;
        spawn(async move {
            if let Err(e) = api::return_submission(&api, &sid).await {
                error.set(Some(format!("{e}")));
            } else {
                on_close.call(());
            }
        });
    };

    let on_close_cancel = props.on_close;
    let on_close_modal = props.on_close;

    rsx! {
        Modal {
            open: true,
            size: ModalSize::Medium,
            title: "Grade submission".to_string(),
            on_close: move |_| on_close_modal.call(()),
            p { class: "submission-grade-modal__attempt", "Attempt #{attempt_number}" }
            if penalty_will_apply {
                p { class: "submission-grade-modal__penalty-note",
                    "This submission is late. A {late_penalty_percent}% penalty will be deducted from the numeric grade you enter."
                }
            }
            if mode == "numeric" {
                if use_rubric {
                    {
                        let r = rubric.clone();
                        rsx! {
                            fieldset { class: "submission-grade-modal__rubric",
                                legend { "{r.as_ref().map(|r| r.title.clone()).unwrap_or_default()}" }
                                if let Some(r) = r.as_ref() {
                                    for c in r.criteria.iter() {
                                        {
                                            let cid = c.id.clone();
                                            let cmax = c.max_points;
                                            let current = criterion_points.read().get(&cid).copied().unwrap_or(0.0);
                                            rsx! {
                                                Field {
                                                    label: format!("{} (max {})", c.label, cmax),
                                                    Input {
                                                        value: format!("{current}"),
                                                        input_type: "number".to_string(),
                                                        on_input: move |v: String| {
                                                            if let Ok(n) = v.parse::<f64>() {
                                                                let mut criterion_points = criterion_points;
                                                                let mut map = criterion_points.read().clone();
                                                                map.insert(cid.clone(), n.clamp(0.0, cmax as f64));
                                                                criterion_points.set(map);
                                                            }
                                                        },
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                p { class: "submission-grade-modal__rubric-total",
                                    "Total: {rubric_total} / {max_points}"
                                }
                            }
                        }
                    }
                } else {
                    Field {
                        label: format!("Numeric grade (max {max_points})"),
                        Input {
                            value: format!("{}", numeric.read().unwrap_or(0.0)),
                            input_type: "number".to_string(),
                            on_input: move |v: String| {
                                if let Ok(n) = v.parse::<f64>() { numeric.set(Some(n)); }
                            },
                        }
                    }
                }
                Field {
                    label: "Letter grade (optional)".to_string(),
                    Input {
                        value: letter.read().clone(),
                        input_type: "text".to_string(),
                        on_input: move |v| letter.set(v),
                    }
                }
            } else {
                fieldset { class: "submission-grade-modal__fieldset",
                    legend { "Pass/fail" }
                    label { class: "submission-grade-modal__radio",
                        Radio {
                            checked: matches!(*passed.read(), Some(true)),
                            name: "passed".to_string(),
                            value: "true".to_string(),
                            on_change: move |_| passed.set(Some(true)),
                        }
                        span { "Pass" }
                    }
                    label { class: "submission-grade-modal__radio",
                        Radio {
                            checked: matches!(*passed.read(), Some(false)),
                            name: "passed".to_string(),
                            value: "false".to_string(),
                            on_change: move |_| passed.set(Some(false)),
                        }
                        span { "Fail" }
                    }
                }
            }
            Field {
                label: "Feedback (visible to student after release)".to_string(),
                for_id: "submission-feedback".to_string(),
                textarea {
                    id: "submission-feedback",
                    name: "feedback",
                    "aria-describedby": "submission-feedback-description",
                    class: "ds-input",
                    rows: 4,
                    value: "{feedback}",
                    oninput: move |e| feedback.set(e.value()),
                }
            }

            FormError { message: error.read().clone() }

            div { class: "modal-actions",
                Button {
                    label: "Save grade".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: on_save,
                }
                if release_mode == "manual" {
                    Button {
                        label: "Release grade".to_string(),
                        variant: ButtonVariant::Primary,
                        on_click: on_save_release,
                    }
                }
                Button {
                    label: "Return for resubmit".to_string(),
                    variant: ButtonVariant::Secondary,
                    on_click: on_return,
                }
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Ghost,
                    on_click: move |_| on_close_cancel.call(()),
                }
            }
        }
    }
}
