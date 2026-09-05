// crates/features-courses/src/lesson_notes.rs
//! Personal study widget for a lesson: the caller's private notes (a textarea
//! that autosaves with PUT on blur) plus a bookmark toggle. Both are scoped to
//! the caller server-side; this is the only UI that reads/writes them.
//!
//! API client functions live here (not in `api.rs`) and call the publicly
//! re-exported `api::fetch_json`, matching the typed-DTO + path-format pattern
//! the announcements widget uses. Reads the API context via `use_api()` so the
//! mount site only needs to pass `lesson_id`.

use crate::api::{self, ApiContext, ApiError};
use design_system::{use_toast_sender, Card, SkeletonLine, ToastLevel};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// API client (mirrors api.rs wrappers; calls the re-exported fetch_json)
// ---------------------------------------------------------------------------

/// Mirrors the backend `NoteResponse` in `crates/backend/src/handlers/notes.rs`.
/// `updated_at` is None when the caller has no stored note yet.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct NoteResponse {
    pub lesson_id: String,
    pub body: String,
    pub updated_at: Option<String>,
}

/// Mirrors the backend `BookmarkStateDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct BookmarkState {
    pub lesson_id: String,
    pub bookmarked: bool,
}

/// Mirrors the backend `BookmarkDto` (the `/v1/me/bookmarks` list rows). Kept
/// here so a future "my bookmarks" page can reuse the same client type.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct BookmarkDto {
    pub lesson_id: String,
    pub course_id: String,
    pub course_slug: String,
    pub lesson_title: String,
    pub created_at: String,
}

#[derive(serde::Serialize)]
struct PutNoteBody<'a> {
    body: &'a str,
}

/// `GET /v1/lessons/{id}/notes` — the caller's own note (empty when unset).
pub async fn get_note(ctx: &ApiContext, lesson_id: &str) -> Result<NoteResponse, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/lessons/{lesson_id}/notes"),
        None::<&()>,
    )
    .await
}

/// `PUT /v1/lessons/{id}/notes` — upsert; an empty body clears the note.
pub async fn put_note(
    ctx: &ApiContext,
    lesson_id: &str,
    body: &str,
) -> Result<NoteResponse, ApiError> {
    api::fetch_json(
        ctx,
        "PUT",
        &format!("/v1/lessons/{lesson_id}/notes"),
        Some(&PutNoteBody { body }),
    )
    .await
}

/// `GET /v1/me/bookmarks` — the caller's bookmarks, newest-first.
pub async fn list_bookmarks(ctx: &ApiContext) -> Result<Vec<BookmarkDto>, ApiError> {
    api::fetch_json(ctx, "GET", "/v1/me/bookmarks", None::<&()>).await
}

/// `POST /v1/lessons/{id}/bookmark` — bookmark the lesson (idempotent).
pub async fn add_bookmark(ctx: &ApiContext, lesson_id: &str) -> Result<BookmarkState, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/lessons/{lesson_id}/bookmark"),
        None::<&()>,
    )
    .await
}

/// `DELETE /v1/lessons/{id}/bookmark` — remove the bookmark (idempotent).
pub async fn remove_bookmark(ctx: &ApiContext, lesson_id: &str) -> Result<BookmarkState, ApiError> {
    api::fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/lessons/{lesson_id}/bookmark"),
        None::<&()>,
    )
    .await
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct LessonNotesProps {
    pub lesson_id: String,
}

#[component]
pub fn LessonNotes(props: LessonNotesProps) -> Element {
    let api = api::use_api();
    let lesson_id = props.lesson_id.clone();

    // Load the existing note + bookmark state once, keyed on lesson_id so a
    // prev/next navigation that swaps the prop re-fetches.
    let note_resource = use_resource({
        let api = api.clone();
        let lesson_id = lesson_id.clone();
        move || {
            let api = api.clone();
            let lesson_id = lesson_id.clone();
            async move { get_note(&api, &lesson_id).await }
        }
    });
    let bookmarks_resource = use_resource({
        let api = api.clone();
        let lesson_id = lesson_id.clone();
        move || {
            let api = api.clone();
            let lesson_id = lesson_id.clone();
            async move {
                list_bookmarks(&api)
                    .await
                    .map(|list| list.iter().any(|b| b.lesson_id == lesson_id))
            }
        }
    });

    // Resolve the bookmark's known state (None while loading or on error) before
    // the rsx so we don't hold the resource read guard inside the macro.
    let bookmark_initial: Option<bool> = bookmarks_resource
        .read_unchecked()
        .as_ref()
        .and_then(|r| r.as_ref().ok().copied());

    rsx! {
        Card {
            div { class: "lesson-notes",
                div { class: "lesson-notes__head",
                    h3 { class: "lesson-notes__title", "Your notes" }
                    BookmarkToggle {
                        api: api.clone(),
                        lesson_id: lesson_id.clone(),
                        initial: bookmark_initial,
                    }
                }
                match &*note_resource.read_unchecked() {
                    Some(Ok(note)) => rsx! {
                        NoteEditor {
                            api: api.clone(),
                            lesson_id: lesson_id.clone(),
                            initial_body: note.body.clone(),
                            initial_updated_at: note.updated_at.clone(),
                        }
                    },
                    Some(Err(e)) => rsx! {
                        p { class: "form-error", "Couldn't load your notes: {e}" }
                    },
                    None => rsx! {
                        SkeletonLine { width: "90%".to_string() }
                        SkeletonLine { width: "70%".to_string() }
                    },
                }
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct NoteEditorProps {
    api: ApiContext,
    lesson_id: String,
    initial_body: String,
    initial_updated_at: Option<String>,
}

#[component]
fn NoteEditor(props: NoteEditorProps) -> Element {
    let api = props.api.clone();
    let lesson_id = props.lesson_id.clone();

    let mut body = use_signal(|| props.initial_body.clone());
    // Track the last value we persisted so blur only PUTs when something changed.
    let mut saved_body = use_signal(|| props.initial_body.clone());
    let mut status = use_signal(|| {
        if props.initial_updated_at.is_some() {
            SaveStatus::Saved
        } else {
            SaveStatus::Idle
        }
    });
    let mut toast = use_toast_sender();

    let on_blur = move |_| {
        let api = api.clone();
        let lesson_id = lesson_id.clone();
        let current = body.read().clone();
        // No change since last save → nothing to do.
        if current == *saved_body.read() {
            return;
        }
        if current.chars().count() > 20_000 {
            status.set(SaveStatus::Error(
                "Notes must be 20,000 characters or fewer.".into(),
            ));
            return;
        }
        status.set(SaveStatus::Saving);
        spawn(async move {
            match put_note(&api, &lesson_id, current.trim_end()).await {
                Ok(resp) => {
                    saved_body.set(resp.body.clone());
                    body.set(resp.body);
                    status.set(SaveStatus::Saved);
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Couldn't save notes", msg.clone());
                    status.set(SaveStatus::Error(msg));
                }
            }
        });
    };

    rsx! {
        textarea {
            class: "ds-input lesson-notes__textarea",
            rows: "6",
            placeholder: "Jot down a private note for this lesson… (saves when you click away)",
            value: "{body}",
            oninput: move |e| body.set(e.value()),
            onblur: on_blur,
        }
        div { class: "lesson-notes__status muted",
            match &*status.read() {
                SaveStatus::Idle => rsx! { span { "Private to you." } },
                SaveStatus::Saving => rsx! { span { "Saving…" } },
                SaveStatus::Saved => rsx! { span { "Saved · private to you." } },
                SaveStatus::Error(msg) => rsx! { span { class: "form-error", "{msg}" } },
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum SaveStatus {
    Idle,
    Saving,
    Saved,
    Error(String),
}

#[derive(Clone, Props, PartialEq)]
struct BookmarkToggleProps {
    api: ApiContext,
    lesson_id: String,
    /// None while the initial state is still loading.
    initial: Option<bool>,
}

#[component]
fn BookmarkToggle(props: BookmarkToggleProps) -> Element {
    let api = props.api.clone();
    let lesson_id = props.lesson_id.clone();
    let initial = props.initial;

    // `None` until the parent resource resolves; track an Option so the button
    // can show a neutral loading label until we know the real state.
    let mut bookmarked = use_signal(|| initial);
    // Reflect a late-arriving initial value (resource resolves after first paint).
    use_effect(move || {
        if bookmarked.peek().is_none() {
            if let Some(v) = initial {
                bookmarked.set(Some(v));
            }
        }
    });
    let mut busy = use_signal(|| false);
    let mut toast = use_toast_sender();

    let on_click = move |_| {
        let api = api.clone();
        let lesson_id = lesson_id.clone();
        let current = bookmarked.peek().unwrap_or(false);
        if *busy.read() {
            return;
        }
        busy.set(true);
        // Optimistic flip; revert on error.
        bookmarked.set(Some(!current));
        spawn(async move {
            let result = if current {
                remove_bookmark(&api, &lesson_id)
                    .await
                    .map(|s| s.bookmarked)
            } else {
                add_bookmark(&api, &lesson_id).await.map(|s| s.bookmarked)
            };
            match result {
                Ok(state) => bookmarked.set(Some(state)),
                Err(e) => {
                    bookmarked.set(Some(current));
                    toast.push(ToastLevel::Danger, "Bookmark failed", format!("{e}"));
                }
            }
            busy.set(false);
        });
    };

    let state = *bookmarked.read();
    let is_on = state.unwrap_or(false);
    let loading = state.is_none();
    let label = match state {
        None => "Bookmark",
        Some(true) => "★ Bookmarked",
        Some(false) => "☆ Bookmark",
    };

    rsx! {
        button {
            r#type: "button",
            class: if is_on { "ds-button ds-button--secondary lesson-notes__bookmark is-on" } else { "ds-button ds-button--ghost lesson-notes__bookmark" },
            disabled: *busy.read() || loading,
            "aria-pressed": "{is_on}",
            onclick: on_click,
            "{label}"
        }
    }
}
