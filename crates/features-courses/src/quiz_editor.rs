//! Teacher quiz authoring (learning-suite Cycle 3): quiz settings (mode,
//! time limit, attempts, module placement, publish) and the full question
//! list across the five prompt shapes. Saving replaces the question set
//! (`PUT …/questions`) and patches the settings.

use crate::api::{self, AuthoringQuestionDto};
use core_types::quiz::{QuizChoice, QuizPrompt};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Field, Input, Select, SelectOption, SkeletonCard,
    ToastLevel,
};
use dioxus::prelude::*;

fn new_choice(n: usize) -> QuizChoice {
    QuizChoice {
        id: format!("c{n}"),
        text: String::new(),
    }
}

fn default_prompt(kind: &str) -> QuizPrompt {
    match kind {
        "multi_select" => QuizPrompt::MultiSelect {
            choices: vec![new_choice(1), new_choice(2)],
            correct: vec![],
        },
        "true_false" => QuizPrompt::TrueFalse { correct: true },
        "ordering" => QuizPrompt::Ordering {
            items: vec![new_choice(1), new_choice(2)],
            correct: vec!["c1".into(), "c2".into()],
        },
        "short_answer" => QuizPrompt::ShortAnswer { accepted: vec![] },
        _ => QuizPrompt::SingleChoice {
            choices: vec![new_choice(1), new_choice(2)],
            correct: "c1".into(),
        },
    }
}

fn prompt_kind(prompt: &QuizPrompt) -> &'static str {
    match prompt {
        QuizPrompt::SingleChoice { .. } => "single_choice",
        QuizPrompt::MultiSelect { .. } => "multi_select",
        QuizPrompt::TrueFalse { .. } => "true_false",
        QuizPrompt::Ordering { .. } => "ordering",
        QuizPrompt::ShortAnswer { .. } => "short_answer",
    }
}

fn kind_options() -> Vec<SelectOption> {
    vec![
        SelectOption {
            value: "single_choice".into(),
            label: "Single choice".into(),
        },
        SelectOption {
            value: "multi_select".into(),
            label: "Multi-select".into(),
        },
        SelectOption {
            value: "true_false".into(),
            label: "True / False".into(),
        },
        SelectOption {
            value: "ordering".into(),
            label: "Ordering".into(),
        },
        SelectOption {
            value: "short_answer".into(),
            label: "Short answer".into(),
        },
    ]
}

#[derive(Props, Clone, PartialEq)]
pub struct QuizEditorProps {
    pub course_id: String,
    pub quiz_id: String,
}

#[component]
pub fn QuizEditor(props: QuizEditorProps) -> Element {
    let cx = api::use_api();
    let toast = use_toast_sender();
    let course_id = props.course_id.clone();
    let quiz_id = props.quiz_id.clone();

    let detail = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        let quiz_id = quiz_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let quiz_id = quiz_id.clone();
            async move { api::get_quiz(&cx, &course_id, &quiz_id).await }
        }
    });
    let outline = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&cx, &course_id).await }
        }
    });

    // Editable state, hydrated once from the loaded quiz.
    let mut hydrated = use_signal(|| false);
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut mode = use_signal(|| "graded".to_string());
    let mut module_id = use_signal(String::new);
    let mut time_limit_minutes = use_signal(String::new);
    let mut max_attempts = use_signal(String::new);
    let mut status = use_signal(|| "draft".to_string());
    let mut questions: Signal<Vec<AuthoringQuestionDto>> = use_signal(Vec::new);
    let mut saving = use_signal(|| false);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    {
        let snap = detail.read_unchecked();
        if let Some(Ok(d)) = snap.as_ref() {
            if !*hydrated.peek() {
                hydrated.set(true);
                title.set(d.quiz.title.clone());
                description.set(d.quiz.description.clone().unwrap_or_default());
                mode.set(d.quiz.mode.clone());
                module_id.set(d.quiz.module_id.clone().unwrap_or_default());
                time_limit_minutes.set(
                    d.quiz
                        .time_limit_seconds
                        .map(|s| (s / 60).to_string())
                        .unwrap_or_default(),
                );
                max_attempts.set(
                    d.quiz
                        .max_attempts
                        .map(|a| a.to_string())
                        .unwrap_or_default(),
                );
                status.set(d.quiz.status.clone());
                questions.set(d.questions.clone().unwrap_or_default());
            }
        }
    }

    if !*hydrated.read() {
        let snap = detail.read_unchecked();
        if let Some(Err(e)) = snap.as_ref() {
            return rsx! { p { class: "error", "Could not load the quiz: {e}" } };
        }
        return rsx! { SkeletonCard {} };
    }

    let module_options: Vec<SelectOption> = {
        let snap = outline.read_unchecked();
        let mut options = vec![SelectOption {
            value: String::new(),
            label: "Course level (no module)".into(),
        }];
        if let Some(Ok(modules)) = snap.as_ref() {
            options.extend(modules.iter().map(|m| SelectOption {
                value: m.id.clone(),
                label: m.title.clone(),
            }));
        }
        options
    };

    let save = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        let quiz_id = quiz_id.clone();
        move |publish: bool| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let quiz_id = quiz_id.clone();
            let qs = questions.peek().clone();
            // Client-side validation mirrors the server's so authors get
            // immediate feedback.
            for (i, q) in qs.iter().enumerate() {
                if q.prompt_text.trim().is_empty() {
                    error.set(Some(format!("Question {} needs prompt text.", i + 1)));
                    return;
                }
                if let Err(e) = q.prompt.validate() {
                    error.set(Some(format!("Question {}: {e}", i + 1)));
                    return;
                }
            }
            if publish && qs.is_empty() {
                error.set(Some("Add at least one question before publishing.".into()));
                return;
            }
            error.set(None);
            saving.set(true);
            let mut toast = toast;
            let status_value = status.peek().clone();
            let patch = serde_json::json!({
                "title": title.peek().trim(),
                "description": description.peek().clone(),
                "mode": mode.peek().clone(),
                "module_id": if module_id.peek().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(module_id.peek().clone())
                },
                "time_limit_seconds": time_limit_minutes.peek().trim().parse::<i32>().ok()
                    .filter(|m| *m > 0).map(|m| m * 60),
                "max_attempts": max_attempts.peek().trim().parse::<i32>().ok()
                    .filter(|a| *a > 0),
                "status": if publish { "published".to_string() } else { status_value },
            });
            spawn(async move {
                let result: Result<(), api::ApiError> = async {
                    api::replace_quiz_questions(&cx, &course_id, &quiz_id, &qs).await?;
                    api::patch_quiz(&cx, &course_id, &quiz_id, &patch).await?;
                    Ok(())
                }
                .await;
                saving.set(false);
                match result {
                    Ok(()) => {
                        if publish {
                            status.set("published".into());
                        }
                        toast.push(
                            ToastLevel::Success,
                            "Saved",
                            if publish {
                                "Quiz published."
                            } else {
                                "Quiz saved."
                            },
                        );
                    }
                    Err(e) => error.set(Some(format!("Save failed: {e}"))),
                }
            });
        }
    };
    let mut save_draft = save.clone();
    let mut save_publish = save;

    let current_status = status.read().clone();
    let question_count = questions.read().len();

    rsx! {
        section { class: "quiz-editor",
            div { class: "quiz-editor-settings",
                Field { label: "Title".to_string(),
                    Input {
                        value: title.read().clone(),
                        on_input: move |v: String| title.set(v),
                    }
                }
                Field { label: "Description".to_string(),
                    Input {
                        value: description.read().clone(),
                        on_input: move |v: String| description.set(v),
                    }
                }
                div { class: "quiz-editor-settings-row",
                    Field { label: "Mode".to_string(),
                        Select {
                            value: mode.read().clone(),
                            options: vec![
                                SelectOption { value: "graded".into(), label: "Graded".into() },
                                SelectOption { value: "practice".into(), label: "Practice".into() },
                            ],
                            on_change: move |v: String| mode.set(v),
                        }
                    }
                    Field { label: "Placement".to_string(),
                        Select {
                            value: module_id.read().clone(),
                            options: module_options,
                            on_change: move |v: String| module_id.set(v),
                        }
                    }
                    Field { label: "Time limit (minutes)".to_string(),
                        helper: Some("Empty = no limit".to_string()),
                        Input {
                            value: time_limit_minutes.read().clone(),
                            input_type: "number".to_string(),
                            on_input: move |v: String| time_limit_minutes.set(v),
                        }
                    }
                    Field { label: "Max attempts".to_string(),
                        helper: Some("Graded only; empty = unlimited".to_string()),
                        Input {
                            value: max_attempts.read().clone(),
                            input_type: "number".to_string(),
                            on_input: move |v: String| max_attempts.set(v),
                        }
                    }
                }
            }

            h3 { class: "quiz-editor-heading", "Questions ({question_count})" }
            for index in 0..question_count {
                QuestionEditor { questions, index }
            }
            div { class: "quiz-editor-add-row",
                Button {
                    label: "Add question".to_string(),
                    variant: ButtonVariant::Secondary,
                    on_click: move |_| {
                        questions.write().push(AuthoringQuestionDto {
                            id: None,
                            prompt_text: String::new(),
                            prompt: default_prompt("single_choice"),
                            explanation: String::new(),
                            points: 1,
                        });
                    },
                }
            }

            if let Some(e) = error.read().as_ref() {
                p { class: "form-error", "{e}" }
            }
            div { class: "quiz-editor-actions",
                Button {
                    label: "Save".to_string(),
                    variant: ButtonVariant::Secondary,
                    loading: *saving.read(),
                    on_click: move |_| save_draft(false),
                }
                if current_status != "published" {
                    Button {
                        label: "Save & publish".to_string(),
                        variant: ButtonVariant::Primary,
                        loading: *saving.read(),
                        on_click: move |_| save_publish(true),
                    }
                } else {
                    span { class: "quiz-editor-status", "Published" }
                }
            }
        }
    }
}

#[component]
fn QuestionEditor(questions: Signal<Vec<AuthoringQuestionDto>>, index: usize) -> Element {
    let Some(question) = questions.read().get(index).cloned() else {
        return rsx! {};
    };
    let kind = prompt_kind(&question.prompt);

    let body: Element = match question.prompt.clone() {
        QuizPrompt::SingleChoice { choices, correct } => {
            choice_editor(questions, index, choices, vec![correct], false)
        }
        QuizPrompt::MultiSelect { choices, correct } => {
            choice_editor(questions, index, choices, correct, true)
        }
        QuizPrompt::TrueFalse { correct } => rsx! {
            Field { label: "Correct answer".to_string(),
                Select {
                    value: if correct { "true".to_string() } else { "false".to_string() },
                    options: vec![
                        SelectOption { value: "true".into(), label: "True".into() },
                        SelectOption { value: "false".into(), label: "False".into() },
                    ],
                    on_change: move |v: String| {
                        if let Some(q) = questions.write().get_mut(index) {
                            q.prompt = QuizPrompt::TrueFalse { correct: v == "true" };
                        }
                    },
                }
            }
        },
        QuizPrompt::Ordering { items, .. } => rsx! {
            p { class: "quiz-editor-hint",
                "Enter the items in the CORRECT order — students see them shuffled."
            }
            for (ci, choice) in items.iter().enumerate() {
                div { class: "quiz-editor-choice", key: "{choice.id}",
                    span { class: "quiz-editor-choice-no", "{ci + 1}." }
                    Input {
                        value: choice.text.clone(),
                        on_input: move |v: String| {
                            if let Some(q) = questions.write().get_mut(index) {
                                if let QuizPrompt::Ordering { items, correct } = &mut q.prompt {
                                    if let Some(c) = items.get_mut(ci) {
                                        c.text = v;
                                    }
                                    *correct = items.iter().map(|c| c.id.clone()).collect();
                                }
                            }
                        },
                    }
                }
            }
            Button {
                label: "Add item".to_string(),
                variant: ButtonVariant::Ghost,
                on_click: move |_| {
                    if let Some(q) = questions.write().get_mut(index) {
                        if let QuizPrompt::Ordering { items, correct } = &mut q.prompt {
                            items.push(new_choice(items.len() + 1));
                            *correct = items.iter().map(|c| c.id.clone()).collect();
                        }
                    }
                },
            }
        },
        QuizPrompt::ShortAnswer { accepted } => rsx! {
            Field { label: "Accepted answers".to_string(),
                for_id: format!("quiz-question-{index}-accepted-answers"),
                helper: Some("One per line; case and spacing are ignored when grading".to_string()),
                textarea {
                    id: format!("quiz-question-{index}-accepted-answers"),
                    name: format!("question_{index}_accepted_answers"),
                    "aria-describedby": format!("quiz-question-{index}-accepted-answers-description"),
                    class: "ds-input quiz-editor-accepted",
                    value: accepted.join("\n"),
                    oninput: move |evt: FormEvent| {
                        if let Some(q) = questions.write().get_mut(index) {
                            q.prompt = QuizPrompt::ShortAnswer {
                                accepted: evt
                                    .value()
                                    .lines()
                                    .map(str::to_string)
                                    .filter(|l| !l.trim().is_empty())
                                    .collect(),
                            };
                        }
                    },
                }
            }
        },
    };

    rsx! {
        div { class: "quiz-editor-question",
            div { class: "quiz-editor-question-head",
                span { class: "quiz-editor-question-no", "Q{index + 1}" }
                Select {
                    value: kind.to_string(),
                    options: kind_options(),
                    on_change: move |v: String| {
                        if let Some(q) = questions.write().get_mut(index) {
                            q.prompt = default_prompt(&v);
                        }
                    },
                }
                Input {
                    value: question.points.to_string(),
                    input_type: "number".to_string(),
                    on_input: move |v: String| {
                        if let Some(q) = questions.write().get_mut(index) {
                            q.points = v.parse().unwrap_or(1).max(1);
                        }
                    },
                }
                span { class: "quiz-editor-points-label", "pts" }
                Button {
                    label: "Remove".to_string(),
                    variant: ButtonVariant::Ghost,
                    on_click: move |_| {
                        questions.write().remove(index);
                    },
                }
            }
            Field { label: "Question".to_string(),
                Input {
                    value: question.prompt_text.clone(),
                    on_input: move |v: String| {
                        if let Some(q) = questions.write().get_mut(index) {
                            q.prompt_text = v;
                        }
                    },
                }
            }
            {body}
        }
    }
}

/// Shared editor for single-choice / multi-select shapes.
fn choice_editor(
    mut questions: Signal<Vec<AuthoringQuestionDto>>,
    index: usize,
    choices: Vec<QuizChoice>,
    correct: Vec<String>,
    multi: bool,
) -> Element {
    rsx! {
        p { class: "quiz-editor-hint", "Tick the correct option(s)." }
        for (ci, choice) in choices.iter().enumerate() {
            div { class: "quiz-editor-choice", key: "{choice.id}",
                input {
                    r#type: if multi { "checkbox" } else { "radio" },
                    checked: correct.contains(&choice.id),
                    onchange: {
                        let choice_id = choice.id.clone();
                        move |_| {
                            if let Some(q) = questions.write().get_mut(index) {
                                match &mut q.prompt {
                                    QuizPrompt::SingleChoice { correct, .. } => {
                                        *correct = choice_id.clone();
                                    }
                                    QuizPrompt::MultiSelect { correct, .. } => {
                                        if correct.contains(&choice_id) {
                                            correct.retain(|c| c != &choice_id);
                                        } else {
                                            correct.push(choice_id.clone());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    },
                }
                Input {
                    value: choice.text.clone(),
                    on_input: move |v: String| {
                        if let Some(q) = questions.write().get_mut(index) {
                            let choices = match &mut q.prompt {
                                QuizPrompt::SingleChoice { choices, .. } => choices,
                                QuizPrompt::MultiSelect { choices, .. } => choices,
                                _ => return,
                            };
                            if let Some(c) = choices.get_mut(ci) {
                                c.text = v;
                            }
                        }
                    },
                }
            }
        }
        Button {
            label: "Add choice".to_string(),
            variant: ButtonVariant::Ghost,
            on_click: move |_| {
                if let Some(q) = questions.write().get_mut(index) {
                    let choices = match &mut q.prompt {
                        QuizPrompt::SingleChoice { choices, .. } => choices,
                        QuizPrompt::MultiSelect { choices, .. } => choices,
                        _ => return,
                    };
                    choices.push(new_choice(choices.len() + 1));
                }
            },
        }
    }
}
