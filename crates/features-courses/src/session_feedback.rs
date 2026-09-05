// crates/features-courses/src/session_feedback.rs
//! Post-session feedback / ratings.
//!
//! Students see a 1..5 star picker + an optional comment box; after submitting
//! (or if they revise) the form collapses to a "thanks, you rated N★" summary.
//! Staff see the aggregate: average, count, and recent comments.
//!
//! Mirrors `announcements` (the API-client-in-module + `use_api()` + Card/Button
//! pattern). The API client functions live here and call the re-exported
//! `api::fetch_json`, matching every other endpoint wrapper.

use crate::api::{self, ApiContext, ApiError};
use design_system::{Button, ButtonVariant, Card, SkeletonLine, ToastLevel};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror crates/backend/src/handlers/session_feedback.rs)
// ---------------------------------------------------------------------------

/// Mirrors the backend `SubmitFeedbackResponse` (flattened `FeedbackDto`).
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FeedbackDto {
    pub id: String,
    pub session_id: String,
    pub user_id: String,
    pub rating: i32,
    pub comment: Option<String>,
    pub created_at: String,
}

/// Mirrors the backend `FeedbackCommentDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FeedbackCommentDto {
    pub rating: i32,
    pub comment: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub created_at: String,
}

/// Mirrors the backend `FeedbackSummaryDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FeedbackSummaryDto {
    pub count: i64,
    pub average: Option<f64>,
    pub recent_comments: Vec<FeedbackCommentDto>,
}

#[derive(serde::Serialize)]
struct SubmitFeedbackBody<'a> {
    rating: i32,
    comment: Option<&'a str>,
}

/// `POST /v1/sessions/{id}/feedback` — enrolled participant; upsert.
pub async fn submit_feedback(
    ctx: &ApiContext,
    session_id: &str,
    rating: i32,
    comment: Option<&str>,
) -> Result<FeedbackDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/sessions/{session_id}/feedback"),
        Some(&SubmitFeedbackBody { rating, comment }),
    )
    .await
}

/// `GET /v1/sessions/{id}/feedback` — staff-only aggregate summary.
pub async fn fetch_summary(
    ctx: &ApiContext,
    session_id: &str,
) -> Result<FeedbackSummaryDto, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/sessions/{session_id}/feedback"),
        None::<&()>,
    )
    .await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const MAX_COMMENT_LEN: usize = 1000;

/// "★★★★☆" for a 0..=5 count. Pure so it's unit-testable.
fn stars_glyphs(filled: i32) -> String {
    let filled = filled.clamp(0, 5) as usize;
    let mut s = String::with_capacity(5);
    for _ in 0..filled {
        s.push('\u{2605}'); // ★
    }
    for _ in filled..5 {
        s.push('\u{2606}'); // ☆
    }
    s
}

/// Author label: display_name, else email, else "Anonymous".
fn author_label(c: &FeedbackCommentDto) -> String {
    if let Some(name) = c.author_display_name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = c.author_email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    "Anonymous".to_string()
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct SessionFeedbackProps {
    pub session_id: String,
    /// Staff see the aggregate summary; students see the rating form.
    pub is_teacher: bool,
}

#[component]
pub fn SessionFeedback(props: SessionFeedbackProps) -> Element {
    if props.is_teacher {
        rsx! { StaffSummary { session_id: props.session_id.clone() } }
    } else {
        rsx! { StudentForm { session_id: props.session_id.clone() } }
    }
}

#[derive(Clone, Props, PartialEq)]
struct StudentFormProps {
    session_id: String,
}

#[component]
fn StudentForm(props: StudentFormProps) -> Element {
    let api = api::use_api();
    let session_id = props.session_id.clone();

    let mut selected = use_signal(|| 0_i32);
    let mut hovered = use_signal(|| 0_i32);
    let mut comment = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    // Once submitted, collapse to a thank-you summary showing the rating.
    let mut submitted = use_signal(|| Option::<i32>::None);
    let mut toast = design_system::use_toast_sender();

    let on_submit = move |_| {
        let api = api.clone();
        let session_id = session_id.clone();
        let rating = *selected.read();
        if !(1..=5).contains(&rating) {
            error.set(Some("Please pick a rating from 1 to 5 stars.".into()));
            return;
        }
        let comment_value = comment.read().trim().to_string();
        if comment_value.chars().count() > MAX_COMMENT_LEN {
            error.set(Some("Comment must be 1000 characters or fewer.".into()));
            return;
        }
        error.set(None);
        submitting.set(true);
        spawn(async move {
            let comment_opt = if comment_value.is_empty() {
                None
            } else {
                Some(comment_value.as_str())
            };
            match submit_feedback(&api, &session_id, rating, comment_opt).await {
                Ok(saved) => {
                    toast.push(
                        ToastLevel::Success,
                        "Thanks for the feedback",
                        "Your rating was recorded.",
                    );
                    submitted.set(Some(saved.rating));
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Couldn't submit feedback", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    };

    // Already submitted: show the collapsed thank-you state.
    if let Some(rating) = *submitted.read() {
        let glyphs = stars_glyphs(rating);
        return rsx! {
            Card {
                div { class: "session-feedback session-feedback--done",
                    h3 { class: "session-feedback__title", "Thanks for rating this class" }
                    div { class: "session-feedback__stars session-feedback__stars--static",
                        span { class: "session-feedback__glyphs", "{glyphs}" }
                        span { class: "muted", " You rated {rating}/5" }
                    }
                }
            }
        };
    }

    let active = (*hovered.read()).max(*selected.read());

    rsx! {
        Card {
            div { class: "session-feedback session-feedback--form",
                h3 { class: "session-feedback__title", "Rate this class" }
                p { class: "muted", "How was the session? Your feedback helps your teacher improve." }
                div {
                    class: "session-feedback__stars",
                    role: "radiogroup",
                    "aria-label": "Star rating",
                    for star in 1..=5 {
                        button {
                            r#type: "button",
                            class: if star <= active { "session-feedback__star session-feedback__star--on" } else { "session-feedback__star" },
                            "aria-label": "{star} star",
                            "aria-pressed": (star <= *selected.read()).to_string(),
                            disabled: *submitting.read(),
                            onmouseenter: move |_| hovered.set(star),
                            onmouseleave: move |_| hovered.set(0),
                            onclick: move |_| selected.set(star),
                            if star <= active { "\u{2605}" } else { "\u{2606}" }
                        }
                    }
                }
                textarea {
                    class: "ds-input session-feedback__comment",
                    placeholder: "Add a comment (optional)",
                    maxlength: "1000",
                    rows: "3",
                    value: "{comment}",
                    disabled: *submitting.read(),
                    oninput: move |e| comment.set(e.value()),
                }
                if let Some(err) = error.read().as_ref() {
                    p { class: "error", "{err}" }
                }
                div { class: "session-feedback__actions",
                    Button {
                        label: if *submitting.read() { "Submitting\u{2026}".to_string() } else { "Submit feedback".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: *submitting.read() || *selected.read() == 0,
                        on_click: on_submit,
                    }
                }
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct StaffSummaryProps {
    session_id: String,
}

#[component]
fn StaffSummary(props: StaffSummaryProps) -> Element {
    let api = api::use_api();
    let session_id = props.session_id.clone();

    let summary = use_resource({
        let api = api.clone();
        let session_id = session_id.clone();
        move || {
            let api = api.clone();
            let session_id = session_id.clone();
            async move { fetch_summary(&api, &session_id).await }
        }
    });

    rsx! {
        Card {
            div { class: "session-feedback session-feedback--summary",
                h3 { class: "session-feedback__title", "Session feedback" }
                match &*summary.read_unchecked() {
                    Some(Ok(s)) if s.count == 0 => rsx! {
                        p { class: "muted", "No feedback yet. Ratings appear here once students respond." }
                    },
                    Some(Ok(s)) => {
                        let avg = s.average.unwrap_or(0.0);
                        let avg_text = format!("{avg:.1}");
                        let glyphs = stars_glyphs(avg.round() as i32);
                        rsx! {
                            div { class: "session-feedback__headline",
                                span { class: "session-feedback__glyphs", "{glyphs}" }
                                span { class: "session-feedback__avg", "{avg_text}/5" }
                                span { class: "muted",
                                    if s.count == 1 { " from 1 rating" } else { " from {s.count} ratings" }
                                }
                            }
                            if !s.recent_comments.is_empty() {
                                ul { class: "session-feedback__comments",
                                    for (i, c) in s.recent_comments.iter().enumerate() {
                                        li { key: "{i}", class: "session-feedback__comment",
                                            div { class: "session-feedback__comment-head muted",
                                                span { class: "session-feedback__glyphs", "{stars_glyphs(c.rating)}" }
                                                span { " \u{2022} " }
                                                span { "{author_label(c)}" }
                                            }
                                            p { class: "session-feedback__comment-body", "{c.comment}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(e)) => rsx! {
                        div { class: "system-state system-state--error", "Couldn't load feedback: {e}" }
                    },
                    None => rsx! {
                        div { class: "system-state system-state--loading",
                            SkeletonLine { width: "60%".to_string() }
                            SkeletonLine { width: "85%".to_string() }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_glyphs_fills_then_empties() {
        assert_eq!(stars_glyphs(0), "\u{2606}\u{2606}\u{2606}\u{2606}\u{2606}");
        assert_eq!(stars_glyphs(3), "\u{2605}\u{2605}\u{2605}\u{2606}\u{2606}");
        assert_eq!(stars_glyphs(5), "\u{2605}\u{2605}\u{2605}\u{2605}\u{2605}");
        // Clamps out-of-range input.
        assert_eq!(stars_glyphs(9), "\u{2605}\u{2605}\u{2605}\u{2605}\u{2605}");
        assert_eq!(stars_glyphs(-2), "\u{2606}\u{2606}\u{2606}\u{2606}\u{2606}");
    }

    #[test]
    fn author_label_prefers_name_then_email_then_anon() {
        let base = FeedbackCommentDto {
            rating: 5,
            comment: "great".into(),
            author_display_name: Some("Ada".into()),
            author_email: Some("ada@example.com".into()),
            created_at: "2026-06-14T10:00:00Z".into(),
        };
        assert_eq!(author_label(&base), "Ada");
        let email_only = FeedbackCommentDto {
            author_display_name: None,
            ..base.clone()
        };
        assert_eq!(author_label(&email_only), "ada@example.com");
        let anon = FeedbackCommentDto {
            author_display_name: None,
            author_email: None,
            ..base
        };
        assert_eq!(author_label(&anon), "Anonymous");
    }
}
