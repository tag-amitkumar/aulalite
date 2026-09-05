// crates/features-courses/src/discussions.rs
//! Course discussion forums / Q&A. A list of threads (pinned-first) with a
//! composer for opening a new thread, plus a thread view that renders nested
//! posts and per-post reply boxes. Everyone enrolled can open threads and
//! reply; staff additionally get pin / lock / delete controls.
//!
//! Mirrors `announcements` for the list/empty/error shell, the MarkdownEditor
//! composer, and the shared safe read-only markdown renderer. API client
//! functions live here and call the publicly
//! re-exported `api::fetch_json`, matching the typed-DTO pattern used elsewhere.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, CardList, EmptyState,
    MarkdownEditor, SkeletonLine, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror crates/backend/src/handlers/discussions.rs)
// ---------------------------------------------------------------------------

/// Mirrors the backend `ThreadDto`. Uuid + timestamps serialize as JSON
/// strings, so they're decoded here as `String`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ThreadDto {
    pub id: String,
    pub course_id: String,
    pub author_user_id: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub pinned: bool,
    pub locked: bool,
    pub reply_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Mirrors the backend `PostDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct PostDto {
    pub id: String,
    pub discussion_id: String,
    pub parent_post_id: Option<String>,
    pub author_user_id: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub body_md: String,
    pub created_at: String,
}

/// Mirrors the backend `ThreadDetailDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ThreadDetailDto {
    pub thread: ThreadDto,
    pub posts: Vec<PostDto>,
}

#[derive(serde::Serialize)]
struct CreateThreadBody<'a> {
    title: &'a str,
    body_md: &'a str,
}

#[derive(serde::Serialize)]
struct CreatePostBody<'a> {
    body_md: &'a str,
    parent_post_id: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct PatchThreadBody {
    pinned: Option<bool>,
    locked: Option<bool>,
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

/// `GET /v1/courses/{cid}/discussions` — pinned-first, newest next.
pub async fn list_threads(ctx: &ApiContext, course_id: &str) -> Result<Vec<ThreadDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/discussions"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{cid}/discussions` — any enrolled member.
pub async fn create_thread(
    ctx: &ApiContext,
    course_id: &str,
    title: &str,
    body_md: &str,
) -> Result<ThreadDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/discussions"),
        Some(&CreateThreadBody { title, body_md }),
    )
    .await
}

/// `GET /v1/discussions/{id}` — thread + posts.
pub async fn get_thread(ctx: &ApiContext, id: &str) -> Result<ThreadDetailDto, ApiError> {
    api::fetch_json(ctx, "GET", &format!("/v1/discussions/{id}"), None::<&()>).await
}

/// `POST /v1/discussions/{id}/posts` — reply (optionally nested).
pub async fn create_post(
    ctx: &ApiContext,
    id: &str,
    body_md: &str,
    parent_post_id: Option<&str>,
) -> Result<PostDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/discussions/{id}/posts"),
        Some(&CreatePostBody {
            body_md,
            parent_post_id,
        }),
    )
    .await
}

/// `PATCH /v1/discussions/{id}` — pin/lock (staff).
pub async fn patch_thread(
    ctx: &ApiContext,
    id: &str,
    pinned: Option<bool>,
    locked: Option<bool>,
) -> Result<ThreadDto, ApiError> {
    api::fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/discussions/{id}"),
        Some(&PatchThreadBody { pinned, locked }),
    )
    .await
}

/// `DELETE /v1/discussions/{id}` — author or staff.
pub async fn delete_thread(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    api::fetch_json::<()>(ctx, "DELETE", &format!("/v1/discussions/{id}"), None::<&()>)
        .await
        .map(|_| ())
}

/// `DELETE /v1/discussions/{id}/posts/{pid}` — author or staff.
pub async fn delete_post(ctx: &ApiContext, id: &str, pid: &str) -> Result<(), ApiError> {
    api::fetch_json::<()>(
        ctx,
        "DELETE",
        &format!("/v1/discussions/{id}/posts/{pid}"),
        None::<&()>,
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Rendering helpers (mirror announcements)
// ---------------------------------------------------------------------------

/// Render markdown read-only through the shared sanitizer.
fn render_markdown(body: &str) -> Element {
    let out = crate::markdown::render_user_markdown_html(body);
    rsx! {
        div { class: "discussion-post__body lesson-md", dangerous_inner_html: "{out}" }
    }
}

/// "Jun 14, 2026 10:39 PM" from RFC3339; the raw string when parsing fails.
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

fn thread_author_label(t: &ThreadDto) -> String {
    label(&t.author_display_name, &t.author_email, &t.author_user_id)
}

fn post_author_label(p: &PostDto) -> String {
    label(&p.author_display_name, &p.author_email, &p.author_user_id)
}

fn label(name: &Option<String>, email: &Option<String>, user_id: &str) -> String {
    if let Some(name) = name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    user_id.chars().take(8).collect()
}

/// Build the (parent_post_id -> children, oldest-first) adjacency the thread
/// view walks to render nested replies. Top-level replies are keyed by `None`.
/// Pure so it's unit-testable.
fn group_children(posts: &[PostDto]) -> std::collections::HashMap<Option<String>, Vec<PostDto>> {
    let mut map: std::collections::HashMap<Option<String>, Vec<PostDto>> =
        std::collections::HashMap::new();
    for p in posts {
        map.entry(p.parent_post_id.clone())
            .or_default()
            .push(p.clone());
    }
    map
}

// ---------------------------------------------------------------------------
// Top-level component: thread list + composer, with an in-place thread view.
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct DiscussionsProps {
    pub course_id: String,
    /// Staff (course owner / teacher / TA / org-admin) get pin / lock / delete
    /// controls; everyone enrolled can open threads and reply.
    pub is_teacher: bool,
}

#[component]
pub fn Discussions(props: DiscussionsProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();
    let is_teacher = props.is_teacher;

    // None = the list view; Some(thread_id) = the thread detail view.
    let mut open_thread = use_signal(|| Option::<String>::None);

    let mut threads = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { list_threads(&api, &course_id).await }
        }
    });

    if let Some(thread_id) = open_thread.read().clone() {
        return rsx! {
            ThreadView {
                api: api.clone(),
                thread_id,
                is_teacher,
                on_back: move |_| {
                    open_thread.set(None);
                    threads.restart();
                },
            }
        };
    }

    rsx! {
        div { class: "discussions motion-page",
            div { class: "discussions__header",
                h2 { "Discussions" }
            }
            ThreadComposer {
                api: api.clone(),
                course_id: course_id.clone(),
                on_posted: move |_| threads.restart(),
            }
            match &*threads.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "No discussions yet".to_string(),
                        description: "Start the conversation above — ask a question or share something with the class.".to_string(),
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for t in items.iter() {
                            ThreadListRow {
                                key: "{t.id}",
                                thread: t.clone(),
                                on_open: move |id: String| open_thread.set(Some(id)),
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load discussions: {e}" }
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
struct ThreadComposerProps {
    api: ApiContext,
    course_id: String,
    on_posted: EventHandler<()>,
}

#[component]
fn ThreadComposer(props: ThreadComposerProps) -> Element {
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
            match create_thread(&api, &course_id, &title_value, &body_value).await {
                Ok(_) => {
                    toast.push(
                        ToastLevel::Success,
                        "Discussion posted",
                        "Your thread is live.",
                    );
                    title.set(String::new());
                    body_md.set(String::new());
                    on_posted.call(());
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Couldn't post discussion", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    };

    rsx! {
        Card {
            div { class: "discussion-composer",
                h3 { class: "discussion-composer__title", "Start a discussion" }
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
                div { class: "discussion-composer__actions",
                    Button {
                        label: if *submitting.read() { "Posting…".to_string() } else { "Post discussion".to_string() },
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
struct ThreadListRowProps {
    thread: ThreadDto,
    on_open: EventHandler<String>,
}

#[component]
fn ThreadListRow(props: ThreadListRowProps) -> Element {
    let t = props.thread.clone();
    let id = t.id.clone();
    let on_open = props.on_open;
    let author = thread_author_label(&t);
    let when = format_ts(&t.created_at);
    let reply_word = if t.reply_count == 1 {
        "reply"
    } else {
        "replies"
    };

    rsx! {
        li { key: "{t.id}", class: "discussion-row",
            Card {
                button {
                    class: "discussion-row__open",
                    r#type: "button",
                    onclick: move |_| on_open.call(id.clone()),
                    div { class: "discussion-row__head",
                        h3 { class: "discussion-row__title", "{t.title}" }
                        div { class: "discussion-row__tags",
                            if t.pinned {
                                Badge { label: "Pinned".to_string(), tone: BadgeTone::Info }
                            }
                            if t.locked {
                                Badge { label: "Locked".to_string(), tone: BadgeTone::Warning }
                            }
                        }
                    }
                    div { class: "discussion-row__meta muted",
                        span { "{author}" }
                        span { " • " }
                        span { "{when}" }
                        span { " • " }
                        span { "{t.reply_count} {reply_word}" }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Thread view: thread body + nested posts + reply boxes.
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
struct ThreadViewProps {
    api: ApiContext,
    thread_id: String,
    is_teacher: bool,
    on_back: EventHandler<()>,
}

#[component]
fn ThreadView(props: ThreadViewProps) -> Element {
    let api = props.api.clone();
    let thread_id = props.thread_id.clone();
    let is_teacher = props.is_teacher;
    let on_back = props.on_back;
    let mut toast = use_toast_sender();

    let mut detail = use_resource({
        let api = api.clone();
        let thread_id = thread_id.clone();
        move || {
            let api = api.clone();
            let thread_id = thread_id.clone();
            async move { get_thread(&api, &thread_id).await }
        }
    });

    let body = match &*detail.read_unchecked() {
        Some(Ok(data)) => {
            let thread = data.thread.clone();
            let posts = data.posts.clone();
            let children = group_children(&posts);
            let top_level = children.get(&None).cloned().unwrap_or_default();
            let locked = thread.locked;
            let author = thread_author_label(&thread);
            let when = format_ts(&thread.created_at);

            // Staff pin/lock/delete on the thread head.
            let pin_label = if thread.pinned { "Unpin" } else { "Pin" };
            let lock_label = if thread.locked { "Unlock" } else { "Lock" };

            let on_toggle_pin = {
                let api = api.clone();
                let id = thread.id.clone();
                let pinned = thread.pinned;
                move |_| {
                    let api = api.clone();
                    let id = id.clone();
                    spawn(async move {
                        match patch_thread(&api, &id, Some(!pinned), None).await {
                            Ok(_) => detail.restart(),
                            Err(e) => {
                                toast.push(ToastLevel::Danger, "Update failed", format!("{e}"))
                            }
                        }
                    });
                }
            };
            let on_toggle_lock = {
                let api = api.clone();
                let id = thread.id.clone();
                let was_locked = thread.locked;
                move |_| {
                    let api = api.clone();
                    let id = id.clone();
                    spawn(async move {
                        match patch_thread(&api, &id, None, Some(!was_locked)).await {
                            Ok(_) => detail.restart(),
                            Err(e) => {
                                toast.push(ToastLevel::Danger, "Update failed", format!("{e}"))
                            }
                        }
                    });
                }
            };
            let on_delete_thread = {
                let api = api.clone();
                let id = thread.id.clone();
                let on_back = on_back;
                move |_| {
                    let api = api.clone();
                    let id = id.clone();
                    spawn(async move {
                        match delete_thread(&api, &id).await {
                            Ok(_) => {
                                toast.push(
                                    ToastLevel::Success,
                                    "Discussion deleted",
                                    "The thread was removed.",
                                );
                                on_back.call(());
                            }
                            Err(e) => {
                                toast.push(ToastLevel::Danger, "Delete failed", format!("{e}"))
                            }
                        }
                    });
                }
            };

            rsx! {
                Card {
                    div { class: "discussion-thread__head",
                        h2 { class: "discussion-thread__title", "{thread.title}" }
                        div { class: "discussion-thread__tags",
                            if thread.pinned {
                                Badge { label: "Pinned".to_string(), tone: BadgeTone::Info }
                            }
                            if thread.locked {
                                Badge { label: "Locked".to_string(), tone: BadgeTone::Warning }
                            }
                        }
                    }
                    div { class: "discussion-thread__meta muted",
                        span { "{author}" }
                        span { " • " }
                        span { "{when}" }
                    }
                    { render_markdown(&thread.body_md) }
                    if is_teacher {
                        div { class: "discussion-thread__staff-actions",
                            Button {
                                label: pin_label.to_string(),
                                variant: ButtonVariant::Ghost,
                                button_type: "button".to_string(),
                                on_click: on_toggle_pin,
                            }
                            Button {
                                label: lock_label.to_string(),
                                variant: ButtonVariant::Ghost,
                                button_type: "button".to_string(),
                                on_click: on_toggle_lock,
                            }
                            Button {
                                label: "Delete thread".to_string(),
                                variant: ButtonVariant::Danger,
                                button_type: "button".to_string(),
                                on_click: on_delete_thread,
                            }
                        }
                    }
                }

                div { class: "discussion-thread__posts",
                    if top_level.is_empty() {
                        p { class: "muted", "No replies yet. Be the first to respond." }
                    }
                    for post in top_level.iter() {
                        PostNode {
                            key: "{post.id}",
                            api: api.clone(),
                            thread_id: thread.id.clone(),
                            post: post.clone(),
                            child_map: children.clone(),
                            depth: 0,
                            is_teacher,
                            locked,
                            on_changed: move |_| detail.restart(),
                        }
                    }
                }

                if locked {
                    p { class: "muted discussion-thread__locked-note", "This thread is locked. New replies are disabled." }
                } else {
                    ReplyBox {
                        api: api.clone(),
                        thread_id: thread.id.clone(),
                        parent_post_id: None,
                        on_posted: move |_| detail.restart(),
                    }
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "system-state system-state--error", "Couldn't load this discussion: {e}" }
        },
        None => rsx! {
            div { class: "system-state system-state--loading",
                SkeletonLine { width: "70%".to_string() }
                SkeletonLine { width: "90%".to_string() }
            }
        },
    };

    rsx! {
        div { class: "discussion-thread motion-page",
            div { class: "discussion-thread__back",
                Button {
                    label: "← Back to discussions".to_string(),
                    variant: ButtonVariant::Ghost,
                    button_type: "button".to_string(),
                    on_click: move |_| on_back.call(()),
                }
            }
            { body }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct PostNodeProps {
    api: ApiContext,
    thread_id: String,
    post: PostDto,
    // NB: not `children` — that name is reserved by Dioxus `#[derive(Props)]`
    // for the special `Element` slot, so the adjacency map needs another name.
    child_map: std::collections::HashMap<Option<String>, Vec<PostDto>>,
    depth: usize,
    is_teacher: bool,
    locked: bool,
    on_changed: EventHandler<()>,
}

#[component]
fn PostNode(props: PostNodeProps) -> Element {
    let api = props.api.clone();
    let thread_id = props.thread_id.clone();
    let post = props.post.clone();
    let children = props.child_map.clone();
    let depth = props.depth;
    let is_teacher = props.is_teacher;
    let locked = props.locked;
    let on_changed = props.on_changed;
    let mut toast = use_toast_sender();
    let mut show_reply = use_signal(|| false);

    let author = post_author_label(&post);
    let when = format_ts(&post.created_at);
    let my_children = children
        .get(&Some(post.id.clone()))
        .cloned()
        .unwrap_or_default();
    // Cap visual nesting indent so deep chains don't run off the edge.
    let indent_depth = depth.min(6);

    let on_delete = {
        let api = api.clone();
        let thread_id = thread_id.clone();
        let post_id = post.id.clone();
        move |_| {
            let api = api.clone();
            let thread_id = thread_id.clone();
            let post_id = post_id.clone();
            spawn(async move {
                match delete_post(&api, &thread_id, &post_id).await {
                    Ok(_) => {
                        toast.push(
                            ToastLevel::Success,
                            "Reply deleted",
                            "The reply was removed.",
                        );
                        on_changed.call(());
                    }
                    Err(e) => toast.push(ToastLevel::Danger, "Delete failed", format!("{e}")),
                }
            });
        }
    };

    rsx! {
        div {
            class: "discussion-post",
            style: "margin-left: {indent_depth}rem;",
            Card {
                div { class: "discussion-post__meta muted",
                    span { "{author}" }
                    span { " • " }
                    span { "{when}" }
                }
                { render_markdown(&post.body_md) }
                div { class: "discussion-post__actions",
                    if !locked {
                        Button {
                            label: if *show_reply.read() { "Cancel".to_string() } else { "Reply".to_string() },
                            variant: ButtonVariant::Ghost,
                            button_type: "button".to_string(),
                            on_click: move |_| { let cur = *show_reply.read(); show_reply.set(!cur); },
                        }
                    }
                    if is_teacher {
                        Button {
                            label: "Delete".to_string(),
                            variant: ButtonVariant::Ghost,
                            button_type: "button".to_string(),
                            on_click: on_delete,
                        }
                    }
                }
                if *show_reply.read() && !locked {
                    ReplyBox {
                        api: api.clone(),
                        thread_id: thread_id.clone(),
                        parent_post_id: Some(post.id.clone()),
                        on_posted: move |_| {
                            show_reply.set(false);
                            on_changed.call(());
                        },
                    }
                }
            }
            for child in my_children.iter() {
                PostNode {
                    key: "{child.id}",
                    api: api.clone(),
                    thread_id: thread_id.clone(),
                    post: child.clone(),
                    child_map: children.clone(),
                    depth: depth + 1,
                    is_teacher,
                    locked,
                    on_changed: on_changed,
                }
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct ReplyBoxProps {
    api: ApiContext,
    thread_id: String,
    parent_post_id: Option<String>,
    on_posted: EventHandler<()>,
}

#[component]
fn ReplyBox(props: ReplyBoxProps) -> Element {
    let api = props.api.clone();
    let thread_id = props.thread_id.clone();
    let parent_post_id = props.parent_post_id.clone();
    let on_posted = props.on_posted;

    let mut body_md = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut toast = use_toast_sender();

    let on_submit = move |_| {
        let api = api.clone();
        let thread_id = thread_id.clone();
        let parent = parent_post_id.clone();
        let body_value = body_md.read().trim().to_string();
        if body_value.is_empty() {
            error.set(Some("Reply can't be empty.".into()));
            return;
        }
        if body_value.chars().count() > 10_000 {
            error.set(Some("Reply must be 10,000 characters or fewer.".into()));
            return;
        }
        error.set(None);
        submitting.set(true);
        spawn(async move {
            match create_post(&api, &thread_id, &body_value, parent.as_deref()).await {
                Ok(_) => {
                    toast.push(ToastLevel::Success, "Reply posted", "Your reply was added.");
                    body_md.set(String::new());
                    on_posted.call(());
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Couldn't post reply", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    };

    rsx! {
        div { class: "discussion-replybox",
            MarkdownEditor {
                value: body_md.read().clone(),
                disabled: *submitting.read(),
                on_change: move |v: String| body_md.set(v),
            }
            if let Some(err) = error.read().as_ref() {
                p { class: "error", "{err}" }
            }
            div { class: "discussion-replybox__actions",
                Button {
                    label: if *submitting.read() { "Posting…".to_string() } else { "Post reply".to_string() },
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    disabled: *submitting.read(),
                    on_click: on_submit,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: &str, parent: Option<&str>) -> PostDto {
        PostDto {
            id: id.into(),
            discussion_id: "d".into(),
            parent_post_id: parent.map(|s| s.to_string()),
            author_user_id: "fedcba9876543210".into(),
            author_display_name: Some("Ada".into()),
            author_email: Some("ada@example.com".into()),
            body_md: "hi".into(),
            created_at: "2026-06-14T10:00:00Z".into(),
        }
    }

    #[test]
    fn group_children_buckets_by_parent() {
        let posts = vec![
            post("p1", None),
            post("p2", None),
            post("p1a", Some("p1")),
            post("p1a1", Some("p1a")),
        ];
        let map = group_children(&posts);
        assert_eq!(map.get(&None).unwrap().len(), 2);
        assert_eq!(map.get(&Some("p1".to_string())).unwrap().len(), 1);
        assert_eq!(map.get(&Some("p1a".to_string())).unwrap().len(), 1);
        assert!(!map.contains_key(&Some("p2".to_string())));
    }

    #[test]
    fn label_prefers_name_then_email_then_short_id() {
        let t = ThreadDto {
            id: "0123456789abcdef".into(),
            course_id: "c".into(),
            author_user_id: "fedcba9876543210".into(),
            author_display_name: Some("Ada Lovelace".into()),
            author_email: Some("ada@example.com".into()),
            title: "Welcome".into(),
            body_md: "# Hi".into(),
            pinned: false,
            locked: false,
            reply_count: 0,
            created_at: "2026-06-14T10:00:00Z".into(),
            updated_at: "2026-06-14T10:00:00Z".into(),
        };
        assert_eq!(thread_author_label(&t), "Ada Lovelace");
        let email_only = ThreadDto {
            author_display_name: None,
            ..t.clone()
        };
        assert_eq!(thread_author_label(&email_only), "ada@example.com");
        let id_only = ThreadDto {
            author_display_name: None,
            author_email: None,
            ..t
        };
        assert_eq!(thread_author_label(&id_only), "fedcba98");
    }

    #[test]
    fn format_ts_falls_back_on_bad_input() {
        assert_eq!(format_ts("not-a-date"), "not-a-date");
        assert_ne!(format_ts("2026-06-14T10:00:00Z"), "2026-06-14T10:00:00Z");
    }
}
