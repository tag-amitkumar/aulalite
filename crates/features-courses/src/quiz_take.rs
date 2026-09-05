//! Student quiz-taking flow (learning-suite Cycle 3): intro → one
//! `QuestionCard` at a time with an optional ticking `QuizTimer` →
//! server-graded submit → `QuizResults`.
//!
//! Grading is server-authoritative: answers are collected locally, the
//! submit endpoint grades against the stored keys (students never receive
//! them), and the result drives the summary view.

use crate::api::{self, QuizDetailDto, QuizSubmitResultDto, StudentQuestionDto};
use core_types::quiz::StudentQuizPrompt;
use design_system::kinetics_ui::{
    QuestionCard, QuizAnswer as KQuizAnswer, QuizChoice as KQuizChoice, QuizPrompt as KQuizPrompt,
    QuizQuestion as KQuizQuestion, QuizResults, QuizTimer,
};
use design_system::{Button, ButtonVariant, SkeletonCard};
use dioxus::prelude::*;
use std::collections::HashMap;

/// Build the kinetics question (which carries an answer key slot) from the
/// student-safe prompt. Keys are placeholders — reveal/grading never happens
/// client-side in this flow.
fn to_kinetics_question(q: &StudentQuestionDto) -> KQuizQuestion {
    let kind = match &q.prompt {
        StudentQuizPrompt::SingleChoice { choices } => KQuizPrompt::SingleChoice {
            choices: choices
                .iter()
                .map(|c| KQuizChoice::new(c.id.clone(), c.text.clone()))
                .collect(),
            correct: String::new(),
        },
        StudentQuizPrompt::MultiSelect { choices } => KQuizPrompt::MultiSelect {
            choices: choices
                .iter()
                .map(|c| KQuizChoice::new(c.id.clone(), c.text.clone()))
                .collect(),
            correct: Vec::new(),
        },
        StudentQuizPrompt::TrueFalse {} => KQuizPrompt::TrueFalse { correct: false },
        StudentQuizPrompt::Ordering { items } => KQuizPrompt::Ordering {
            items: items
                .iter()
                .map(|c| KQuizChoice::new(c.id.clone(), c.text.clone()))
                .collect(),
            correct: Vec::new(),
        },
        StudentQuizPrompt::ShortAnswer {} => KQuizPrompt::ShortAnswer {
            accepted: Vec::new(),
        },
    };
    KQuizQuestion::new(q.id.clone(), q.prompt_text.clone(), kind)
}

/// Kinetics answer → the shared submission vocabulary.
fn to_api_answer(answer: &KQuizAnswer) -> core_types::quiz::QuizAnswer {
    match answer {
        KQuizAnswer::Choice(v) => core_types::quiz::QuizAnswer::Choice(v.clone()),
        KQuizAnswer::Choices(v) => core_types::quiz::QuizAnswer::Choices(v.clone()),
        KQuizAnswer::Bool(v) => core_types::quiz::QuizAnswer::Bool(*v),
        KQuizAnswer::Order(v) => core_types::quiz::QuizAnswer::Order(v.clone()),
        KQuizAnswer::Text(v) => core_types::quiz::QuizAnswer::Text(v.clone()),
    }
}

#[derive(Clone, PartialEq)]
enum TakeState {
    Intro,
    Taking { attempt_id: String },
    Submitting,
    Done(QuizSubmitResultDto),
}

#[derive(Props, Clone, PartialEq)]
pub struct QuizTakeProps {
    pub course_id: String,
    pub quiz_id: String,
}

#[component]
pub fn QuizTake(props: QuizTakeProps) -> Element {
    let cx = api::use_api();
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

    let state = use_signal(|| TakeState::Intro);
    let answers: Signal<HashMap<String, KQuizAnswer>> = use_signal(HashMap::new);
    let current_index = use_signal(|| 0usize);
    let remaining_seconds: Signal<Option<u32>> = use_signal(|| None);
    let error: Signal<Option<String>> = use_signal(|| None);

    let snap = detail.read_unchecked();
    let quiz: QuizDetailDto = match snap.as_ref() {
        Some(Ok(d)) => d.clone(),
        Some(Err(e)) => return rsx! { p { class: "error", "Could not load the quiz: {e}" } },
        None => return rsx! { SkeletonCard {} },
    };
    drop(snap);

    let questions: Vec<StudentQuestionDto> = quiz.student_questions.clone().unwrap_or_default();
    let total_points: i32 = questions.iter().map(|q| q.points).sum();
    let time_limit = quiz.quiz.time_limit_seconds.map(|s| s.max(0) as u32);

    // Shared submit routine (Submit button + timer expiry).
    let do_submit = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        let quiz_id = quiz_id.clone();
        let questions = questions.clone();
        let mut state = state;
        let answers = answers;
        let mut error = error;
        move || {
            let TakeState::Taking { attempt_id } = state.peek().clone() else {
                return;
            };
            state.set(TakeState::Submitting);
            let cx = cx.clone();
            let course_id = course_id.clone();
            let quiz_id = quiz_id.clone();
            let payload: Vec<serde_json::Value> = questions
                .iter()
                .filter_map(|q| {
                    answers.peek().get(&q.id).map(|a| {
                        serde_json::json!({
                            "question_id": q.id,
                            "answer": to_api_answer(a),
                        })
                    })
                })
                .collect();
            spawn(async move {
                let body = serde_json::json!({ "answers": payload });
                match api::submit_quiz_attempt(&cx, &course_id, &quiz_id, &attempt_id, &body).await
                {
                    Ok(result) => state.set(TakeState::Done(result)),
                    Err(e) => {
                        error.set(Some(format!("Submitting failed: {e}")));
                        state.set(TakeState::Taking { attempt_id });
                    }
                }
            });
        }
    };

    // Tick the timer once per second while taking; auto-submit at zero.
    #[cfg(target_arch = "wasm32")]
    {
        let mut remaining = remaining_seconds;
        let state_t = state;
        let do_submit_t = do_submit.clone();
        use_effect(move || {
            let taking = matches!(*state_t.read(), TakeState::Taking { .. });
            if !taking {
                return;
            }
            let mut do_submit_t = do_submit_t.clone();
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(1_000).await;
                    if !matches!(*state_t.peek(), TakeState::Taking { .. }) {
                        break;
                    }
                    let Some(left) = *remaining.peek() else { break };
                    if left <= 1 {
                        remaining.set(Some(0));
                        do_submit_t();
                        break;
                    }
                    remaining.set(Some(left - 1));
                }
            });
        });
    }

    let start_attempt = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        let quiz_id = quiz_id.clone();
        let mut state = state;
        let mut answers = answers;
        let mut current_index = current_index;
        let mut remaining = remaining_seconds;
        let mut error = error;
        move |_| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let quiz_id = quiz_id.clone();
            spawn(async move {
                match api::start_quiz_attempt(&cx, &course_id, &quiz_id).await {
                    Ok(attempt) => {
                        answers.set(HashMap::new());
                        current_index.set(0);
                        remaining.set(time_limit);
                        error.set(None);
                        state.set(TakeState::Taking {
                            attempt_id: attempt.id,
                        });
                    }
                    Err(e) => error.set(Some(format!("Could not start the quiz: {e}"))),
                }
            });
        }
    };

    let error_line: Element = match error.read().as_ref() {
        Some(e) => rsx! { p { class: "form-error", "{e}" } },
        None => rsx! {},
    };

    let body = match state.read().clone() {
        TakeState::Intro => {
            let attempts_line = match (quiz.quiz.mode.as_str(), quiz.quiz.max_attempts) {
                ("practice", _) => "Practice quiz — unlimited attempts.".to_string(),
                (_, Some(cap)) => format!(
                    "Graded quiz — attempt {} of {cap}.",
                    quiz.quiz.my_submitted_attempts + 1
                ),
                (_, None) => "Graded quiz.".to_string(),
            };
            let attempts_left = quiz
                .quiz
                .max_attempts
                .map(|cap| {
                    quiz.quiz.mode != "graded" || quiz.quiz.my_submitted_attempts < cap as i64
                })
                .unwrap_or(true);
            rsx! {
                div { class: "quiz-intro",
                    p { class: "quiz-intro-meta",
                        "{questions.len()} questions · {total_points} points"
                        if let Some(limit) = time_limit {
                            " · {limit / 60} min limit"
                        }
                    }
                    p { class: "quiz-intro-attempts", "{attempts_line}" }
                    if let (Some(best), Some(max)) = (quiz.quiz.my_best_score, quiz.quiz.my_best_max) {
                        p { class: "quiz-intro-best", "Best score so far: {best}/{max}" }
                    }
                    if attempts_left {
                        Button {
                            label: if quiz.quiz.my_submitted_attempts > 0 { "Retake quiz".to_string() } else { "Start quiz".to_string() },
                            variant: ButtonVariant::Primary,
                            on_click: start_attempt,
                        }
                    } else {
                        p { class: "quiz-intro-capped", "You've used all attempts for this quiz." }
                    }
                }
            }
        }
        TakeState::Taking { .. } | TakeState::Submitting => {
            let submitting = matches!(*state.read(), TakeState::Submitting);
            let idx = (*current_index.read()).min(questions.len().saturating_sub(1));
            let question = &questions[idx];
            let kinetics_question = to_kinetics_question(question);
            let current_answer = answers.read().get(&question.id).cloned();
            let answered = answers.read().len();
            let counter = format!("Question {} of {}", idx + 1, questions.len());
            let qid = question.id.clone();
            let mut answers_w = answers;
            let mut index_w = current_index;
            let last = idx + 1 >= questions.len();
            let mut do_submit_btn = do_submit.clone();
            rsx! {
                div { class: "quiz-take",
                    if let (Some(total), Some(left)) = (time_limit, *remaining_seconds.read()) {
                        QuizTimer { total_seconds: total, remaining_seconds: left }
                    }
                    QuestionCard {
                        question: kinetics_question,
                        answer: current_answer,
                        revealed: false,
                        counter,
                        on_answer: move |a: KQuizAnswer| {
                            answers_w.write().insert(qid.clone(), a);
                        },
                    }
                    div { class: "quiz-take-nav",
                        Button {
                            label: "← Previous".to_string(),
                            variant: ButtonVariant::Ghost,
                            disabled: idx == 0 || submitting,
                            on_click: move |_| {
                                index_w.set(idx.saturating_sub(1));
                            },
                        }
                        span { class: "quiz-take-progress", "{answered}/{questions.len()} answered" }
                        if last {
                            Button {
                                label: "Submit quiz".to_string(),
                                variant: ButtonVariant::Primary,
                                loading: submitting,
                                on_click: move |_| do_submit_btn(),
                            }
                        } else {
                            Button {
                                label: "Next →".to_string(),
                                variant: ButtonVariant::Secondary,
                                disabled: submitting,
                                on_click: move |_| {
                                    index_w.set(idx + 1);
                                },
                            }
                        }
                    }
                }
            }
        }
        TakeState::Done(result) => {
            let correct = result.per_question.iter().filter(|q| q.correct).count();
            let per_question: Vec<bool> = result.per_question.iter().map(|q| q.correct).collect();
            let can_retry = quiz.quiz.mode == "practice"
                || quiz
                    .quiz
                    .max_attempts
                    .map(|cap| quiz.quiz.my_submitted_attempts + 1 < cap as i64)
                    .unwrap_or(true);
            let mut state_w = state;
            rsx! {
                div { class: "quiz-done",
                    QuizResults {
                        correct,
                        total: result.per_question.len(),
                        per_question,
                        on_retry: if can_retry {
                            Some(EventHandler::new(move |_| state_w.set(TakeState::Intro)))
                        } else {
                            None
                        },
                    }
                    p { class: "quiz-done-points",
                        "Score: {result.score_points}/{result.max_points} points"
                    }
                }
            }
        }
    };

    rsx! {
        section { class: "quiz-surface",
            h2 { class: "quiz-title", "{quiz.quiz.title}" }
            if let Some(desc) = quiz.quiz.description.as_ref() {
                p { class: "quiz-description", "{desc}" }
            }
            {error_line}
            {body}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn student_prompts_map_to_kinetics_shapes_with_empty_keys() {
        let q = StudentQuestionDto {
            id: "q1".into(),
            prompt_text: "Pick".into(),
            prompt: StudentQuizPrompt::SingleChoice {
                choices: vec![
                    core_types::quiz::QuizChoice {
                        id: "a".into(),
                        text: "Alpha".into(),
                    },
                    core_types::quiz::QuizChoice {
                        id: "b".into(),
                        text: "Beta".into(),
                    },
                ],
            },
            points: 1,
        };
        let k = to_kinetics_question(&q);
        match k.kind {
            KQuizPrompt::SingleChoice { choices, correct } => {
                assert_eq!(choices.len(), 2);
                assert!(correct.is_empty(), "placeholder key must stay empty");
            }
            other => panic!("wrong shape: {other:?}"),
        }
    }

    #[test]
    fn kinetics_answers_serialize_to_the_shared_vocabulary() {
        let json = serde_json::to_value(to_api_answer(&KQuizAnswer::Choices(vec![
            "a".into(),
            "b".into(),
        ])))
        .unwrap();
        assert_eq!(json["type"], "choices");
        assert_eq!(json["value"][0], "a");
    }
}
