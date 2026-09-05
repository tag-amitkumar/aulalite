//! Shared quiz vocabulary (learning-suite Cycle 3).
//!
//! Mirrors the dioxus-kinetics `ui-learn` quiz shapes (`QuizPrompt`,
//! `QuizAnswer`, `grade_answer`, `normalize_short_answer`) with serde
//! derives so the same JSON flows through Postgres (`quiz_questions.prompt`
//! jsonb), the REST API, and the frontend. Grading is server-authoritative:
//! the backend grades with these functions; the kinetics copies only drive
//! optimistic UI. Keep the semantics in lockstep with
//! `ui-learn/src/quiz.rs`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuizChoice {
    pub id: String,
    pub text: String,
}

/// A question's answer shape INCLUDING its key. Never serialize this to
/// students pre-submission — use [`StudentQuizPrompt`] instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuizPrompt {
    /// Pick exactly one choice.
    SingleChoice {
        choices: Vec<QuizChoice>,
        correct: String,
    },
    /// Pick every correct choice (set equality).
    MultiSelect {
        choices: Vec<QuizChoice>,
        correct: Vec<String>,
    },
    TrueFalse {
        correct: bool,
    },
    /// Arrange the items into `correct` order.
    Ordering {
        items: Vec<QuizChoice>,
        correct: Vec<String>,
    },
    /// Free text, graded against `accepted` after normalization.
    ShortAnswer {
        accepted: Vec<String>,
    },
}

/// The student-safe projection of [`QuizPrompt`]: same shapes, no keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StudentQuizPrompt {
    SingleChoice { choices: Vec<QuizChoice> },
    MultiSelect { choices: Vec<QuizChoice> },
    TrueFalse {},
    Ordering { items: Vec<QuizChoice> },
    ShortAnswer {},
}

impl QuizPrompt {
    /// Strip the answer key for delivery to a student taking the quiz.
    pub fn student_view(&self) -> StudentQuizPrompt {
        match self {
            QuizPrompt::SingleChoice { choices, .. } => StudentQuizPrompt::SingleChoice {
                choices: choices.clone(),
            },
            QuizPrompt::MultiSelect { choices, .. } => StudentQuizPrompt::MultiSelect {
                choices: choices.clone(),
            },
            QuizPrompt::TrueFalse { .. } => StudentQuizPrompt::TrueFalse {},
            QuizPrompt::Ordering { items, .. } => StudentQuizPrompt::Ordering {
                items: items.clone(),
            },
            QuizPrompt::ShortAnswer { .. } => StudentQuizPrompt::ShortAnswer {},
        }
    }

    /// Validation for authoring: every prompt must have a gradable key and
    /// well-formed choices. Returns a user-facing reason on failure.
    pub fn validate(&self) -> Result<(), String> {
        fn check_choices(choices: &[QuizChoice], what: &str) -> Result<(), String> {
            if choices.len() < 2 {
                return Err(format!("{what} needs at least two choices"));
            }
            let mut ids: Vec<&str> = choices.iter().map(|c| c.id.as_str()).collect();
            ids.sort();
            ids.dedup();
            if ids.len() != choices.len() {
                return Err(format!("{what} has duplicate choice ids"));
            }
            if choices.iter().any(|c| c.text.trim().is_empty()) {
                return Err(format!("{what} has an empty choice"));
            }
            Ok(())
        }
        match self {
            QuizPrompt::SingleChoice { choices, correct } => {
                check_choices(choices, "single-choice question")?;
                if !choices.iter().any(|c| &c.id == correct) {
                    return Err("the correct choice is not among the choices".into());
                }
                Ok(())
            }
            QuizPrompt::MultiSelect { choices, correct } => {
                check_choices(choices, "multi-select question")?;
                if correct.is_empty() {
                    return Err("multi-select needs at least one correct choice".into());
                }
                if !correct.iter().all(|id| choices.iter().any(|c| &c.id == id)) {
                    return Err("a correct choice is not among the choices".into());
                }
                Ok(())
            }
            QuizPrompt::TrueFalse { .. } => Ok(()),
            QuizPrompt::Ordering { items, correct } => {
                check_choices(items, "ordering question")?;
                let mut want: Vec<&str> = items.iter().map(|c| c.id.as_str()).collect();
                let mut got: Vec<&str> = correct.iter().map(String::as_str).collect();
                want.sort();
                got.sort();
                if want != got {
                    return Err("the correct order must use every item exactly once".into());
                }
                Ok(())
            }
            QuizPrompt::ShortAnswer { accepted } => {
                if accepted.is_empty() || accepted.iter().all(|a| a.trim().is_empty()) {
                    return Err("short answer needs at least one accepted answer".into());
                }
                Ok(())
            }
        }
    }
}

/// A learner's response, mirroring the [`QuizPrompt`] variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum QuizAnswer {
    Choice(String),
    Choices(Vec<String>),
    Bool(bool),
    Order(Vec<String>),
    Text(String),
}

/// Lowercase, trim, and collapse internal whitespace (kinetics semantics).
pub fn normalize_short_answer(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Grades an answer against its prompt. `None` when the answer shape does
/// not match the prompt kind (a wiring bug, not a wrong answer).
pub fn grade_answer(prompt: &QuizPrompt, answer: &QuizAnswer) -> Option<bool> {
    match (prompt, answer) {
        (QuizPrompt::SingleChoice { correct, .. }, QuizAnswer::Choice(picked)) => {
            Some(picked == correct)
        }
        (QuizPrompt::MultiSelect { correct, .. }, QuizAnswer::Choices(picked)) => {
            let mut want = correct.clone();
            let mut got = picked.clone();
            want.sort();
            want.dedup();
            got.sort();
            got.dedup();
            Some(want == got)
        }
        (QuizPrompt::TrueFalse { correct }, QuizAnswer::Bool(picked)) => Some(picked == correct),
        (QuizPrompt::Ordering { correct, .. }, QuizAnswer::Order(order)) => Some(order == correct),
        (QuizPrompt::ShortAnswer { accepted }, QuizAnswer::Text(text)) => {
            let normalized = normalize_short_answer(text);
            Some(
                accepted
                    .iter()
                    .any(|candidate| normalize_short_answer(candidate) == normalized),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Vec<QuizChoice> {
        vec![
            QuizChoice {
                id: "a".into(),
                text: "Alpha".into(),
            },
            QuizChoice {
                id: "b".into(),
                text: "Beta".into(),
            },
        ]
    }

    #[test]
    fn grades_every_prompt_shape() {
        let single = QuizPrompt::SingleChoice {
            choices: choices(),
            correct: "a".into(),
        };
        assert_eq!(
            grade_answer(&single, &QuizAnswer::Choice("a".into())),
            Some(true)
        );
        assert_eq!(
            grade_answer(&single, &QuizAnswer::Choice("b".into())),
            Some(false)
        );

        let multi = QuizPrompt::MultiSelect {
            choices: choices(),
            correct: vec!["a".into(), "b".into()],
        };
        assert_eq!(
            grade_answer(&multi, &QuizAnswer::Choices(vec!["b".into(), "a".into()])),
            Some(true)
        );
        assert_eq!(
            grade_answer(&multi, &QuizAnswer::Choices(vec!["a".into()])),
            Some(false)
        );

        let tf = QuizPrompt::TrueFalse { correct: true };
        assert_eq!(grade_answer(&tf, &QuizAnswer::Bool(true)), Some(true));

        let ord = QuizPrompt::Ordering {
            items: choices(),
            correct: vec!["b".into(), "a".into()],
        };
        assert_eq!(
            grade_answer(&ord, &QuizAnswer::Order(vec!["b".into(), "a".into()])),
            Some(true)
        );

        let short = QuizPrompt::ShortAnswer {
            accepted: vec!["The Mitochondria".into()],
        };
        assert_eq!(
            grade_answer(&short, &QuizAnswer::Text("  the  mitochondria ".into())),
            Some(true)
        );

        // Shape mismatch is None, not false.
        assert_eq!(grade_answer(&tf, &QuizAnswer::Text("true".into())), None);
    }

    #[test]
    fn student_view_strips_answer_keys() {
        let prompt = QuizPrompt::SingleChoice {
            choices: choices(),
            correct: "a".into(),
        };
        let json = serde_json::to_string(&prompt.student_view()).unwrap();
        assert!(!json.contains("correct"), "leaked key: {json}");
        assert!(json.contains("Alpha"));

        let short = QuizPrompt::ShortAnswer {
            accepted: vec!["secret".into()],
        };
        let json = serde_json::to_string(&short.student_view()).unwrap();
        assert!(!json.contains("secret"), "leaked key: {json}");
    }

    #[test]
    fn prompt_validation_rejects_malformed_keys() {
        assert!(QuizPrompt::SingleChoice {
            choices: choices(),
            correct: "missing".into(),
        }
        .validate()
        .is_err());
        assert!(QuizPrompt::MultiSelect {
            choices: choices(),
            correct: vec![],
        }
        .validate()
        .is_err());
        assert!(QuizPrompt::Ordering {
            items: choices(),
            correct: vec!["a".into()],
        }
        .validate()
        .is_err());
        assert!(QuizPrompt::ShortAnswer { accepted: vec![] }
            .validate()
            .is_err());
        assert!(QuizPrompt::TrueFalse { correct: false }.validate().is_ok());
    }

    #[test]
    fn prompt_json_round_trips_with_kind_tag() {
        let prompt = QuizPrompt::Ordering {
            items: choices(),
            correct: vec!["a".into(), "b".into()],
        };
        let json = serde_json::to_value(&prompt).unwrap();
        assert_eq!(json["kind"], "ordering");
        let back: QuizPrompt = serde_json::from_value(json).unwrap();
        assert_eq!(back, prompt);
    }
}
