//! Course quiz list (learning-suite Cycle 3). Students see published
//! quizzes with their best score and a take/retake action; teachers also
//! see drafts, a create action, and per-quiz edit links.

use crate::api::{self, QuizDto};
use design_system::{Badge, BadgeTone, Button, ButtonVariant, EmptyState, SkeletonCard};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct QuizListProps {
    pub course_id: String,
    pub can_author: bool,
    pub can_take: bool,
    pub is_staff: bool,
    pub on_open: EventHandler<String>,
    pub on_edit: EventHandler<String>,
}

#[component]
pub fn QuizList(props: QuizListProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();

    let mut quizzes = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::list_quizzes(&cx, &course_id).await }
        }
    });

    let mut creating = use_signal(|| false);
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let on_edit = props.on_edit;
    let create_quiz = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        move |_| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            creating.set(true);
            spawn(async move {
                let body = api::CreateQuizBody {
                    title: "Untitled quiz".into(),
                    description: None,
                    mode: "graded".into(),
                    module_id: None,
                    time_limit_seconds: None,
                    max_attempts: None,
                };
                match api::create_quiz(&cx, &course_id, &body).await {
                    Ok(quiz) => on_edit.call(quiz.id),
                    Err(e) => error.set(Some(format!("Could not create the quiz: {e}"))),
                }
                creating.set(false);
                quizzes.restart();
            });
        }
    };

    let snap = quizzes.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(list)) if list.is_empty() => rsx! {
            EmptyState {
                title: "No quizzes yet.".to_string(),
                description: if props.can_author {
                    "Create the course's first quiz to check understanding.".to_string()
                } else if props.is_staff {
                    "No quizzes have been created for this course yet.".to_string()
                } else {
                    "Your teacher hasn't published any quizzes yet.".to_string()
                },
            }
        },
        Some(Ok(list)) => {
            let rows = list.clone();
            rsx! {
                ul { class: "quiz-list",
                    for quiz in rows {
                        QuizListRow {
                            quiz: quiz.clone(),
                            can_author: props.can_author,
                            can_take: props.can_take,
                            is_staff: props.is_staff,
                            on_open: props.on_open,
                            on_edit: props.on_edit,
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load quizzes: {e}" } },
        None => rsx! { SkeletonCard {} },
    };
    drop(snap);

    rsx! {
        section { class: "quiz-list-surface",
            if props.can_author {
                div { class: "quiz-list-actions",
                    Button {
                        label: "New quiz".to_string(),
                        variant: ButtonVariant::Primary,
                        loading: *creating.read(),
                        on_click: create_quiz,
                    }
                }
            }
            if let Some(e) = error.read().as_ref() {
                p { class: "form-error", "{e}" }
            }
            {body}
        }
    }
}

#[component]
fn QuizListRow(
    quiz: QuizDto,
    can_author: bool,
    can_take: bool,
    is_staff: bool,
    on_open: EventHandler<String>,
    on_edit: EventHandler<String>,
) -> Element {
    let mode_tone = if quiz.mode == "practice" {
        BadgeTone::Info
    } else {
        BadgeTone::Primary
    };
    let status_tone = match quiz.status.as_str() {
        "published" => BadgeTone::Success,
        "archived" => BadgeTone::Neutral,
        _ => BadgeTone::Warning,
    };
    let quiz_id_open = quiz.id.clone();
    let quiz_id_edit = quiz.id.clone();

    rsx! {
        li { class: "quiz-list-row", key: "{quiz.id}",
            div { class: "quiz-list-row-main",
                span { class: "quiz-list-row-title", "{quiz.title}" }
                div { class: "quiz-list-row-meta",
                    Badge { label: quiz.mode.clone(), tone: mode_tone }
                    if is_staff {
                        Badge { label: quiz.status.clone(), tone: status_tone }
                    }
                    span { class: "quiz-list-row-count", "{quiz.question_count} questions" }
                    if let (Some(best), Some(max)) = (quiz.my_best_score, quiz.my_best_max) {
                        span { class: "quiz-list-row-best", "Best: {best}/{max}" }
                    }
                }
            }
            div { class: "quiz-list-row-actions",
                if can_take && quiz.status == "published" {
                    Button {
                        label: if quiz.my_submitted_attempts > 0 { "Retake".to_string() } else { "Take quiz".to_string() },
                        variant: ButtonVariant::Secondary,
                        on_click: move |_| on_open.call(quiz_id_open.clone()),
                    }
                }
                if can_author {
                    Button {
                        label: "Edit".to_string(),
                        variant: ButtonVariant::Ghost,
                        on_click: move |_| on_edit.call(quiz_id_edit.clone()),
                    }
                }
            }
        }
    }
}
