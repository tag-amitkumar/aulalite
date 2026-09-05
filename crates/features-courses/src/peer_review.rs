// crates/features-courses/src/peer_review.rs
//! Assignment peer-review surface.
//!
//! Renders on the assignment-detail page. Three audiences, one component:
//!   * Students see their reviewer queue (allocations assigned to them) and, per
//!     allocation, a review form (a numeric/comment scorecard).
//!   * The same student, lower on the page, sees the reviews they RECEIVED on
//!     their own submission (reviewer identity hidden when the config is
//!     anonymous).
//!   * Staff see the aggregate (every allocation, filed or not).
//!
//! Mirrors `discussions` for the list/empty/error shell, the MarkdownEditor
//! comment box, and the typed-DTO + `api::fetch_json` client pattern. The score
//! payload is intentionally free-form (`serde_json::Value`) so it can carry
//! either a single numeric score or per-rubric-criterion scores without a
//! bespoke contract.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, CardList, EmptyState,
    MarkdownEditor, SkeletonLine, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror crates/backend/src/handlers/peer_review.rs)
// ---------------------------------------------------------------------------

/// Mirrors the backend `ConfigDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct PeerReviewConfigDto {
    pub assignment_id: String,
    pub course_id: String,
    pub reviews_per_student: i32,
    pub rubric_id: Option<String>,
    pub anonymous: bool,
    pub due_at: Option<String>,
}

/// Mirrors the backend `AllocationDto`. `reviewer_user_id` / `author_user_id`
/// are present only when the caller is allowed to see them. `scores` is the
/// free-form score payload.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AllocationDto {
    pub id: String,
    pub assignment_id: String,
    pub submission_id: String,
    pub status: String,
    #[serde(default)]
    pub reviewer_user_id: Option<String>,
    #[serde(default)]
    pub author_user_id: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub scores: Option<serde_json::Value>,
    #[serde(default)]
    pub comment_md: Option<String>,
}

#[derive(serde::Serialize)]
struct SubmitReviewBody {
    scores: serde_json::Value,
    comment_md: String,
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

/// `GET /v1/assignments/{aid}/peer-review` (config) — we fetch it via the
/// reviewer queue + received endpoints; this convenience reads the summary for
/// staff. There is no standalone GET-config route, so callers infer config
/// presence from the queue/received responses (404 → not configured).
pub async fn fetch_my_queue(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<Vec<AllocationDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/assignments/{assignment_id}/peer-review/mine"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/assignments/{aid}/peer-review/received` — the author's received reviews.
pub async fn fetch_received(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<Vec<AllocationDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/assignments/{assignment_id}/peer-review/received"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/assignments/{aid}/peer-review/summary` — staff aggregate.
pub async fn fetch_summary(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<Vec<AllocationDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/assignments/{assignment_id}/peer-review/summary"),
        None::<&()>,
    )
    .await
}

/// `PUT /v1/assignments/{aid}/peer-review` — staff create/replace config.
pub async fn put_config(
    ctx: &ApiContext,
    assignment_id: &str,
    reviews_per_student: i32,
    anonymous: bool,
) -> Result<PeerReviewConfigDto, ApiError> {
    #[derive(serde::Serialize)]
    struct Body {
        reviews_per_student: i32,
        anonymous: bool,
    }
    api::fetch_json(
        ctx,
        "PUT",
        &format!("/v1/assignments/{assignment_id}/peer-review"),
        Some(&Body {
            reviews_per_student,
            anonymous,
        }),
    )
    .await
}

/// `POST /v1/assignments/{aid}/peer-review/allocate` — staff round-robin.
pub async fn allocate(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<serde_json::Value, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/assignments/{assignment_id}/peer-review/allocate"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/peer-review/allocations/{id}` — reviewer submits/updates a review.
pub async fn submit_review(
    ctx: &ApiContext,
    allocation_id: &str,
    score: f64,
    comment_md: &str,
) -> Result<AllocationDto, ApiError> {
    // A single overall numeric score keyed as "overall". Rubric-aware scoring
    // (per-criterion) is a follow-up; the backend accepts either shape.
    let scores = serde_json::json!({ "overall": score });
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/peer-review/allocations/{allocation_id}"),
        Some(&SubmitReviewBody {
            scores,
            comment_md: comment_md.to_string(),
        }),
    )
    .await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// "Jun 14, 2026 10:39 PM" from RFC3339; the raw string when parsing fails.
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Render markdown read-only through the shared sanitizer.
fn render_markdown(body: &str) -> Element {
    let out = crate::markdown::render_user_markdown_html(body);
    rsx! {
        div { class: "peer-review__body lesson-md", dangerous_inner_html: "{out}" }
    }
}

/// Extract a human label for the overall score from a free-form score payload.
fn score_label(scores: &Option<serde_json::Value>) -> Option<String> {
    let v = scores.as_ref()?;
    if let Some(overall) = v.get("overall").and_then(|x| x.as_f64()) {
        return Some(format!("Score: {overall}"));
    }
    // Fallback: sum any numeric values present.
    if let Some(obj) = v.as_object() {
        let total: f64 = obj.values().filter_map(|x| x.as_f64()).sum();
        if !obj.is_empty() {
            return Some(format!("Score: {total}"));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Top-level component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct PeerReviewProps {
    pub assignment_id: String,
    /// Staff (course owner / teacher / TA / org-admin) configure + allocate and
    /// see the aggregate; students see their queue + received reviews.
    pub is_teacher: bool,
}

#[component]
pub fn PeerReview(props: PeerReviewProps) -> Element {
    if props.is_teacher {
        rsx! {
            StaffPeerReview { assignment_id: props.assignment_id.clone() }
        }
    } else {
        rsx! {
            StudentPeerReview { assignment_id: props.assignment_id.clone() }
        }
    }
}

// ---------------------------------------------------------------------------
// Staff: configure + allocate + aggregate
// ---------------------------------------------------------------------------

#[component]
fn StaffPeerReview(assignment_id: String) -> Element {
    let api = api::use_api();
    let mut reviews_per = use_signal(|| 2i32);
    let mut anonymous = use_signal(|| true);
    let mut busy = use_signal(|| false);
    let mut toast = use_toast_sender();

    let mut summary = use_resource({
        let api = api.clone();
        let assignment_id = assignment_id.clone();
        move || {
            let api = api.clone();
            let assignment_id = assignment_id.clone();
            async move { fetch_summary(&api, &assignment_id).await }
        }
    });

    let on_save_config = {
        let api = api.clone();
        let assignment_id = assignment_id.clone();
        move |_| {
            let api = api.clone();
            let assignment_id = assignment_id.clone();
            let n = *reviews_per.read();
            let anon = *anonymous.read();
            busy.set(true);
            spawn(async move {
                match put_config(&api, &assignment_id, n, anon).await {
                    Ok(_) => toast.push(
                        ToastLevel::Success,
                        "Peer review enabled",
                        "Configuration saved.",
                    ),
                    Err(e) => {
                        toast.push(ToastLevel::Danger, "Couldn't save config", format!("{e}"))
                    }
                }
                busy.set(false);
            });
        }
    };

    let on_allocate = {
        let api = api.clone();
        let assignment_id = assignment_id.clone();
        move |_| {
            let api = api.clone();
            let assignment_id = assignment_id.clone();
            busy.set(true);
            spawn(async move {
                match allocate(&api, &assignment_id).await {
                    Ok(res) => {
                        let n = res
                            .get("allocations_created")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                        toast.push(
                            ToastLevel::Success,
                            "Reviewers allocated",
                            format!("{n} review assignment(s) created."),
                        );
                        summary.restart();
                    }
                    Err(e) => toast.push(ToastLevel::Danger, "Allocation failed", format!("{e}")),
                }
                busy.set(false);
            });
        }
    };

    rsx! {
        div { class: "peer-review peer-review--staff motion-page",
            Card {
                div { class: "peer-review__config",
                    h3 { "Peer review" }
                    p { class: "muted",
                        "Each turned-in submission is assigned to N distinct reviewers (never the author)."
                    }
                    label { class: "peer-review__field",
                        span { "Reviews per student" }
                        input {
                            class: "ds-input",
                            r#type: "number",
                            min: "1",
                            max: "10",
                            value: "{reviews_per}",
                            disabled: *busy.read(),
                            oninput: move |e| {
                                if let Ok(v) = e.value().parse::<i32>() {
                                    reviews_per.set(v.clamp(1, 10));
                                }
                            },
                        }
                    }
                    label { class: "peer-review__field peer-review__field--check",
                        input {
                            r#type: "checkbox",
                            checked: *anonymous.read(),
                            disabled: *busy.read(),
                            onchange: move |e| anonymous.set(e.checked()),
                        }
                        span { "Hide reviewer identity from the author (anonymous)" }
                    }
                    div { class: "peer-review__actions",
                        Button {
                            label: "Save configuration".to_string(),
                            variant: ButtonVariant::Primary,
                            button_type: "button".to_string(),
                            disabled: *busy.read(),
                            on_click: on_save_config,
                        }
                        Button {
                            label: "Allocate reviewers".to_string(),
                            variant: ButtonVariant::Secondary,
                            button_type: "button".to_string(),
                            disabled: *busy.read(),
                            on_click: on_allocate,
                        }
                    }
                }
            }

            h3 { class: "peer-review__section-title", "Allocation summary" }
            match &*summary.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "No allocations yet".to_string(),
                        description: "Save a configuration, then click \"Allocate reviewers\" once submissions are in.".to_string(),
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for a in items.iter() {
                            StaffAllocationRow { key: "{a.id}", alloc: a.clone() }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load summary: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "70%".to_string() }
                        SkeletonLine { width: "90%".to_string() }
                    }
                },
            }
        }
    }
}

#[component]
fn StaffAllocationRow(alloc: AllocationDto) -> Element {
    let reviewer = alloc
        .reviewer_user_id
        .as_ref()
        .map(|s| s.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "—".to_string());
    let author = alloc
        .author_user_id
        .as_ref()
        .map(|s| s.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "—".to_string());
    let submitted = alloc.status == "submitted";
    let score = score_label(&alloc.scores);

    rsx! {
        li { key: "{alloc.id}", class: "peer-review-row",
            Card {
                div { class: "peer-review-row__head",
                    span { class: "peer-review-row__pair", "{reviewer} → {author}" }
                    if submitted {
                        Badge { label: "Reviewed".to_string(), tone: BadgeTone::Success }
                    } else {
                        Badge { label: "Pending".to_string(), tone: BadgeTone::Neutral }
                    }
                }
                if let Some(score) = score {
                    p { class: "peer-review-row__score", "{score}" }
                }
                if let Some(comment) = alloc.comment_md.as_ref().filter(|c| !c.is_empty()) {
                    { render_markdown(comment) }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Student: reviewer queue + received reviews
// ---------------------------------------------------------------------------

#[component]
fn StudentPeerReview(assignment_id: String) -> Element {
    let api = api::use_api();

    let mut queue = use_resource({
        let api = api.clone();
        let assignment_id = assignment_id.clone();
        move || {
            let api = api.clone();
            let assignment_id = assignment_id.clone();
            async move { fetch_my_queue(&api, &assignment_id).await }
        }
    });
    let received = use_resource({
        let api = api.clone();
        let assignment_id = assignment_id.clone();
        move || {
            let api = api.clone();
            let assignment_id = assignment_id.clone();
            async move { fetch_received(&api, &assignment_id).await }
        }
    });

    rsx! {
        div { class: "peer-review peer-review--student motion-page",
            h3 { class: "peer-review__section-title", "Reviews to complete" }
            match &*queue.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "Nothing to review".to_string(),
                        description: "You'll see peer submissions here once your teacher allocates reviewers.".to_string(),
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for a in items.iter() {
                            ReviewerQueueItem {
                                key: "{a.id}",
                                api: api.clone(),
                                alloc: a.clone(),
                                on_submitted: move |_| queue.restart(),
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load your queue: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "70%".to_string() }
                        SkeletonLine { width: "90%".to_string() }
                    }
                },
            }

            h3 { class: "peer-review__section-title", "Reviews you received" }
            match &*received.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    p { class: "muted", "No peer reviews of your work yet." }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for a in items.iter() {
                            ReceivedReviewItem { key: "{a.id}", alloc: a.clone() }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load received reviews: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "80%".to_string() }
                    }
                },
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct ReviewerQueueItemProps {
    api: ApiContext,
    alloc: AllocationDto,
    on_submitted: EventHandler<()>,
}

#[component]
fn ReviewerQueueItem(props: ReviewerQueueItemProps) -> Element {
    let api = props.api.clone();
    let alloc = props.alloc.clone();
    let on_submitted = props.on_submitted;
    let already = alloc.status == "submitted";

    // Seed the form from any prior review so a reviewer can revise.
    let seed_score = alloc
        .scores
        .as_ref()
        .and_then(|v| v.get("overall"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let mut score = use_signal(|| seed_score);
    let mut comment = use_signal(|| alloc.comment_md.clone().unwrap_or_default());
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut toast = use_toast_sender();

    let on_submit = {
        let api = api.clone();
        let allocation_id = alloc.id.clone();
        move |_| {
            let api = api.clone();
            let allocation_id = allocation_id.clone();
            let score_value = *score.read();
            let comment_value = comment.read().trim().to_string();
            if comment_value.chars().count() > 10_000 {
                error.set(Some("Comment must be 10,000 characters or fewer.".into()));
                return;
            }
            error.set(None);
            submitting.set(true);
            spawn(async move {
                match submit_review(&api, &allocation_id, score_value, &comment_value).await {
                    Ok(_) => {
                        toast.push(
                            ToastLevel::Success,
                            "Review submitted",
                            "Thanks for your feedback.",
                        );
                        on_submitted.call(());
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        toast.push(ToastLevel::Danger, "Couldn't submit review", msg.clone());
                        error.set(Some(msg));
                    }
                }
                submitting.set(false);
            });
        }
    };

    let sub_short: String = alloc.submission_id.chars().take(8).collect();

    rsx! {
        li { key: "{alloc.id}", class: "peer-review-row",
            Card {
                div { class: "peer-review-row__head",
                    span { class: "peer-review-row__pair", "Submission {sub_short}" }
                    if already {
                        Badge { label: "Reviewed".to_string(), tone: BadgeTone::Success }
                    } else {
                        Badge { label: "To do".to_string(), tone: BadgeTone::Warning }
                    }
                }
                a {
                    class: "peer-review-row__open",
                    href: "/submissions/{alloc.submission_id}",
                    "Open the submission"
                }
                label { class: "peer-review__field",
                    span { "Overall score" }
                    input {
                        class: "ds-input",
                        r#type: "number",
                        min: "0",
                        step: "0.5",
                        value: "{score}",
                        disabled: *submitting.read(),
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse::<f64>() {
                                score.set(v.max(0.0));
                            }
                        },
                    }
                }
                MarkdownEditor {
                    value: comment.read().clone(),
                    disabled: *submitting.read(),
                    on_change: move |v: String| comment.set(v),
                }
                if let Some(err) = error.read().as_ref() {
                    p { class: "error", "{err}" }
                }
                div { class: "peer-review__actions",
                    Button {
                        label: if already { "Update review".to_string() } else { "Submit review".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: *submitting.read(),
                        on_click: on_submit,
                    }
                }
            }
        }
    }
}

#[component]
fn ReceivedReviewItem(alloc: AllocationDto) -> Element {
    let reviewer = match alloc.reviewer_user_id.as_ref() {
        Some(id) => id.chars().take(8).collect::<String>(),
        None => "Anonymous reviewer".to_string(),
    };
    let when = alloc
        .submitted_at
        .as_ref()
        .map(|s| format_ts(s))
        .unwrap_or_default();
    let score = score_label(&alloc.scores);

    rsx! {
        li { key: "{alloc.id}", class: "peer-review-row",
            Card {
                div { class: "peer-review-row__meta muted",
                    span { "{reviewer}" }
                    if !when.is_empty() {
                        span { " • " }
                        span { "{when}" }
                    }
                }
                if let Some(score) = score {
                    p { class: "peer-review-row__score", "{score}" }
                }
                if let Some(comment) = alloc.comment_md.as_ref().filter(|c| !c.is_empty()) {
                    { render_markdown(comment) }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_label_reads_overall() {
        let v = Some(serde_json::json!({ "overall": 8.5 }));
        assert_eq!(score_label(&v), Some("Score: 8.5".to_string()));
    }

    #[test]
    fn score_label_sums_criteria() {
        let v = Some(serde_json::json!({ "a": 3, "b": 4 }));
        assert_eq!(score_label(&v), Some("Score: 7".to_string()));
    }

    #[test]
    fn score_label_none_when_empty() {
        assert_eq!(score_label(&None), None);
        assert_eq!(score_label(&Some(serde_json::json!({}))), None);
    }

    #[test]
    fn format_ts_falls_back_on_bad_input() {
        assert_eq!(format_ts("not-a-date"), "not-a-date");
        assert_ne!(format_ts("2026-06-14T10:00:00Z"), "2026-06-14T10:00:00Z");
    }
}
