// crates/features-courses/src/announcements.rs
//! Course announcements: staff get a MarkdownEditor composer + the list;
//! students get the list only. Mirrors `assignment_list` (the list/empty/error
//! shell) and `assignment_editor` (the MarkdownEditor composer). Body markdown
//! is rendered read-only through the shared safe markdown renderer.
//!
//! The API client functions live here (not in `api.rs`) and call the publicly
//! re-exported `api::fetch_json`, matching the typed-DTO + path-format pattern
//! every other endpoint wrapper uses.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, CardList, EmptyState, MarkdownEditor,
    SkeletonLine, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// API client (mirrors api.rs wrappers; calls the re-exported fetch_json)
// ---------------------------------------------------------------------------

/// Mirrors the backend `AnnouncementDto` in
/// `crates/backend/src/handlers/announcements.rs`. Uuid + timestamps serialize
/// as JSON strings, so they're decoded here as `String`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AnnouncementDto {
    pub id: String,
    pub course_id: String,
    pub author_user_id: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(serde::Serialize)]
struct CreateAnnouncementBody<'a> {
    title: &'a str,
    body_md: &'a str,
}

/// `GET /v1/courses/{cid}/announcements` — newest-first, visibility-scoped.
pub async fn list_announcements(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<AnnouncementDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/announcements"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{cid}/announcements` — staff-only.
pub async fn create_announcement(
    ctx: &ApiContext,
    course_id: &str,
    title: &str,
    body_md: &str,
) -> Result<AnnouncementDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/announcements"),
        Some(&CreateAnnouncementBody { title, body_md }),
    )
    .await
}

/// `DELETE /v1/courses/{cid}/announcements/{id}` — author or staff.
pub async fn delete_announcement(
    ctx: &ApiContext,
    course_id: &str,
    id: &str,
) -> Result<(), ApiError> {
    api::fetch_json::<()>(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/announcements/{id}"),
        None::<&()>,
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Render announcement markdown read-only through the shared sanitizer.
fn render_markdown(body: &str) -> Element {
    let out = crate::markdown::render_user_markdown_html(body);
    rsx! {
        div { class: "announcement-card__body lesson-md", dangerous_inner_html: "{out}" }
    }
}

/// "Jun 14, 2026 10:39 PM" from RFC3339; the raw string when parsing fails.
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Author label: display_name, else email, else a short slice of the user id.
fn author_label(a: &AnnouncementDto) -> String {
    if let Some(name) = a.author_display_name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = a.author_email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    a.author_user_id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct AnnouncementsProps {
    pub course_id: String,
    /// Staff (course owner / teacher / TA / org-admin) see the composer + a
    /// delete control on each card; students see the read-only list.
    pub is_teacher: bool,
}

#[component]
pub fn Announcements(props: AnnouncementsProps) -> Element {
    // Read the API context like the sibling tabs (QuizList, CourseLeaderboard)
    // rather than taking it as a prop, so the route dispatcher just passes
    // course_id + is_teacher.
    let api = api::use_api();
    let course_id = props.course_id.clone();
    let is_teacher = props.is_teacher;

    let mut announcements = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { list_announcements(&api, &course_id).await }
        }
    });

    rsx! {
        div { class: "announcements motion-page",
            div { class: "announcements__header",
                h2 { "Announcements" }
            }
            if is_teacher {
                AnnouncementComposer {
                    api: api.clone(),
                    course_id: course_id.clone(),
                    on_posted: move |_| announcements.restart(),
                }
            }
            match &*announcements.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "No announcements yet".to_string(),
                        description: if is_teacher {
                            "Post an update above and enrolled students will be notified.".to_string()
                        } else {
                            "Your teachers haven't posted any announcements yet.".to_string()
                        },
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for a in items.iter() {
                            AnnouncementRow {
                                key: "{a.id}",
                                api: api.clone(),
                                course_id: course_id.clone(),
                                announcement: a.clone(),
                                can_delete: is_teacher,
                                on_deleted: move |_| announcements.restart(),
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load announcements: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "70%".to_string() }
                        SkeletonLine { width: "90%".to_string() }
                        SkeletonLine { width: "60%".to_string() }
                    }
                },
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct AnnouncementComposerProps {
    api: ApiContext,
    course_id: String,
    on_posted: EventHandler<()>,
}

#[component]
fn AnnouncementComposer(props: AnnouncementComposerProps) -> Element {
    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let on_posted = props.on_posted;

    let mut title = use_signal(String::new);
    let mut body_md = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut toast = use_toast_sender();

    let on_submit = move |_| {
        let api = api.clone();
        let course_id = course_id.clone();
        let title_value = title.read().trim().to_string();
        let body_value = body_md.read().trim().to_string();
        if title_value.is_empty() {
            error.set(Some("Title is required.".into()));
            return;
        }
        if title_value.chars().count() > 200 {
            error.set(Some("Title must be 200 characters or fewer.".into()));
            return;
        }
        if body_value.is_empty() {
            error.set(Some("Body is required.".into()));
            return;
        }
        if body_value.chars().count() > 10_000 {
            error.set(Some("Body must be 10,000 characters or fewer.".into()));
            return;
        }
        error.set(None);
        submitting.set(true);
        spawn(async move {
            match create_announcement(&api, &course_id, &title_value, &body_value).await {
                Ok(_) => {
                    toast.push(
                        ToastLevel::Success,
                        "Announcement posted",
                        "Enrolled students were notified.",
                    );
                    title.set(String::new());
                    body_md.set(String::new());
                    on_posted.call(());
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(
                        ToastLevel::Danger,
                        "Couldn't post announcement",
                        msg.clone(),
                    );
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    };

    rsx! {
        Card {
            div { class: "announcement-composer",
                h3 { class: "announcement-composer__title", "New announcement" }
                input {
                    class: "ds-input",
                    r#type: "text",
                    placeholder: "Title",
                    maxlength: "200",
                    value: "{title}",
                    disabled: *submitting.read(),
                    oninput: move |e| title.set(e.value()),
                }
                MarkdownEditor {
                    value: body_md.read().clone(),
                    disabled: *submitting.read(),
                    on_change: move |v: String| body_md.set(v),
                }
                if let Some(err) = error.read().as_ref() {
                    p { class: "error", "{err}" }
                }
                div { class: "announcement-composer__actions",
                    Button {
                        label: if *submitting.read() { "Posting…".to_string() } else { "Post announcement".to_string() },
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

#[derive(Clone, Props, PartialEq)]
struct AnnouncementRowProps {
    api: ApiContext,
    course_id: String,
    announcement: AnnouncementDto,
    can_delete: bool,
    on_deleted: EventHandler<()>,
}

#[component]
fn AnnouncementRow(props: AnnouncementRowProps) -> Element {
    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let a = props.announcement.clone();
    let id = a.id.clone();
    let mut deleting = use_signal(|| false);
    let mut toast = use_toast_sender();

    let author = author_label(&a);
    let when = format_ts(&a.created_at);

    let on_delete = move |_| {
        let api = api.clone();
        let course_id = course_id.clone();
        let id = id.clone();
        let on_deleted = props.on_deleted;
        deleting.set(true);
        spawn(async move {
            match delete_announcement(&api, &course_id, &id).await {
                Ok(_) => {
                    toast.push(
                        ToastLevel::Success,
                        "Announcement deleted",
                        "The post was removed.",
                    );
                    on_deleted.call(());
                }
                Err(e) => {
                    toast.push(ToastLevel::Danger, "Delete failed", format!("{e}"));
                    deleting.set(false);
                }
            }
        });
    };

    rsx! {
        li { key: "{a.id}", class: "announcement-card",
            Card {
                div { class: "announcement-card__head",
                    h3 { class: "announcement-card__title", "{a.title}" }
                    if props.can_delete {
                        Button {
                            label: if *deleting.read() { "Deleting…".to_string() } else { "Delete".to_string() },
                            variant: ButtonVariant::Ghost,
                            button_type: "button".to_string(),
                            disabled: *deleting.read(),
                            on_click: on_delete,
                        }
                    }
                }
                div { class: "announcement-card__meta muted",
                    span { "{author}" }
                    span { " • " }
                    span { "{when}" }
                }
                { render_markdown(&a.body_md) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AnnouncementDto {
        AnnouncementDto {
            id: "0123456789abcdef".into(),
            course_id: "c".into(),
            author_user_id: "fedcba9876543210".into(),
            author_display_name: Some("Ada Lovelace".into()),
            author_email: Some("ada@example.com".into()),
            title: "Welcome".into(),
            body_md: "# Hi".into(),
            created_at: "2026-06-14T10:00:00Z".into(),
            updated_at: "2026-06-14T10:00:00Z".into(),
        }
    }

    #[test]
    fn author_label_prefers_name_then_email_then_short_id() {
        let a = sample();
        assert_eq!(author_label(&a), "Ada Lovelace");
        let email_only = AnnouncementDto {
            author_display_name: None,
            ..sample()
        };
        assert_eq!(author_label(&email_only), "ada@example.com");
        let id_only = AnnouncementDto {
            author_display_name: None,
            author_email: None,
            ..sample()
        };
        assert_eq!(author_label(&id_only), "fedcba98");
    }

    #[test]
    fn format_ts_falls_back_on_bad_input() {
        assert_eq!(format_ts("not-a-date"), "not-a-date");
        assert_ne!(format_ts("2026-06-14T10:00:00Z"), "2026-06-14T10:00:00Z");
    }
}
