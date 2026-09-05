// crates/features-courses/src/assignment_editor.rs
//! Create/edit form for assignments. Teacher only.

use crate::api::{
    self, ApiContext, AssignmentDto, CreateAssignmentBody, PatchAssignmentBody,
    UpdateAssignmentBody,
};
use crate::file_picker::{validation, FilePicker};
use design_system::{
    use_toast_sender, Button, ButtonVariant, DateTimePicker, Field, FormError, Input, Radio,
    Select, SelectOption, Switch, ToastLevel,
};
use dioxus::prelude::*;

/// Normalize a `<input type=datetime-local>` value (e.g. `2026-05-30T14:30`,
/// which carries no seconds and no zone) into an RFC3339 UTC timestamp the
/// backend accepts. Empty input → `None` (clears the due date). Appends `:00`
/// for missing seconds and a trailing `Z` if no zone is present.
fn datetime_local_to_rfc3339(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Already zoned (ends in Z or has a +/- offset after the time)? Leave it.
    if trimmed.ends_with('Z') || trimmed.ends_with('z') {
        return Some(trimmed.to_string());
    }
    // datetime-local is "YYYY-MM-DDTHH:MM" or "YYYY-MM-DDTHH:MM:SS".
    // Count the colons after the date to decide if seconds are present.
    let time_part = trimmed.split('T').nth(1).unwrap_or("");
    let has_seconds = time_part.matches(':').count() >= 2;
    let with_seconds = if has_seconds {
        trimmed.to_string()
    } else {
        format!("{trimmed}:00")
    };
    Some(format!("{with_seconds}Z"))
}

/// Convert a stored RFC3339 `due_at` back into the `datetime-local` value the
/// picker expects (`YYYY-MM-DDTHH:MM`). Best-effort: trims any seconds/zone.
fn rfc3339_to_datetime_local(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('Z').trim_end_matches('z');
    // Keep date + HH:MM only.
    match trimmed.split_once('T') {
        Some((date, time)) => {
            let hhmm: String = time.split(':').take(2).collect::<Vec<_>>().join(":");
            format!("{date}T{hhmm}")
        }
        None => trimmed.to_string(),
    }
}

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentEditorProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub course_id: String,
    pub initial: Option<AssignmentDto>,
}

pub fn AssignmentEditor(props: AssignmentEditorProps) -> Element {
    let initial = props.initial.clone();
    use_context_provider(|| props.api.clone());
    let mut title = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.title.clone())
            .unwrap_or_default()
    });
    let mut instructions = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.instructions_md.clone())
            .unwrap_or_default()
    });
    let mut grading_mode = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.grading_mode.clone())
            .unwrap_or_else(|| "numeric".into())
    });
    let mut max_points = use_signal(|| initial.as_ref().and_then(|a| a.max_points).unwrap_or(100));
    let mut allow_late = use_signal(|| initial.as_ref().map(|a| a.allow_late).unwrap_or(true));
    let mut lock_on_submit =
        use_signal(|| initial.as_ref().map(|a| a.lock_on_submit).unwrap_or(false));
    let mut accepts_text = use_signal(|| initial.as_ref().map(|a| a.accepts_text).unwrap_or(true));
    let mut accepts_files =
        use_signal(|| initial.as_ref().map(|a| a.accepts_files).unwrap_or(true));
    let mut release_mode = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.release_mode.clone())
            .unwrap_or_else(|| "instant".into())
    });
    let mut late_penalty_percent = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.late_penalty_percent)
            .unwrap_or(0)
    });
    let mut max_resubmissions =
        use_signal(|| initial.as_ref().map(|a| a.max_resubmissions).unwrap_or(0));
    // Due date is held as a `datetime-local` value (no zone). Convert from the
    // stored RFC3339 timestamp on load and back to RFC3339 UTC on save.
    let mut due_at = use_signal(|| {
        initial
            .as_ref()
            .and_then(|a| a.due_at.as_ref())
            .map(|d| rfc3339_to_datetime_local(d))
            .unwrap_or_default()
    });
    let mut attachment_asset_ids = use_signal(|| {
        initial
            .as_ref()
            .map(|a| a.attachment_asset_ids.clone())
            .unwrap_or_default()
    });
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut saving = use_signal(|| false);
    let mut toast = use_toast_sender();

    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let course_slug = props.course_slug.clone();
    let existing_assignment_id = initial.as_ref().map(|a| a.id.clone());

    // Fetch any rubric already attached to this (existing) assignment so the
    // builder can prefill. Only meaningful for a saved assignment; for a fresh
    // draft there is no id yet, so the resource resolves to `None`.
    let rubric_api = api.clone();
    let rubric_aid = existing_assignment_id.clone();
    let rubric_res = use_resource(move || {
        let api = rubric_api.clone();
        let aid = rubric_aid.clone();
        async move {
            match aid {
                Some(aid) => api::get_assignment_rubric(&api, &aid).await,
                None => Ok(None),
            }
        }
    });

    let save_api = api.clone();
    let save_assignment_id = existing_assignment_id.clone();
    let on_save = move |_| {
        let api = save_api.clone();
        let course_id = course_id.clone();
        let course_slug = course_slug.clone();
        let existing_id = save_assignment_id.clone();
        spawn(async move {
            saving.set(true);
            let title_v = title.read().clone();
            let instructions_v = instructions.read().clone();
            let grading_mode_v = grading_mode.read().clone();
            let release_mode_v = release_mode.read().clone();
            let max_points_v = if grading_mode_v == "numeric" {
                Some(*max_points.read())
            } else {
                None
            };
            let due_at_v = datetime_local_to_rfc3339(&due_at.read());

            if let Some(assignment_id) = existing_id {
                // Edit mode: PATCH the existing assignment instead of creating a
                // duplicate.
                let body = UpdateAssignmentBody {
                    title: &title_v,
                    instructions_md: &instructions_v,
                    grading_mode: &grading_mode_v,
                    max_points: max_points_v,
                    allow_late: *allow_late.read(),
                    lock_on_submit: *lock_on_submit.read(),
                    accepts_text: *accepts_text.read(),
                    accepts_files: *accepts_files.read(),
                    release_mode: &release_mode_v,
                    late_penalty_percent: *late_penalty_percent.read(),
                    max_resubmissions: *max_resubmissions.read(),
                    due_at: due_at_v,
                    lesson_id: None,
                };
                match api::update_assignment(&api, &assignment_id, &body).await {
                    Ok(a) => {
                        toast.push(
                            ToastLevel::Success,
                            "Assignment updated",
                            "Your changes were saved.",
                        );
                        #[cfg(target_arch = "wasm32")]
                        if let Some(win) = web_sys::window() {
                            let _ = win
                                .location()
                                .set_href(&format!("/courses/{course_slug}/assignments/{}", a.id));
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        let _ = (a, course_slug);
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        toast.push(ToastLevel::Danger, "Update failed", msg.clone());
                        error.set(Some(msg));
                    }
                }
            } else {
                let body = CreateAssignmentBody {
                    title: &title_v,
                    instructions_md: &instructions_v,
                    grading_mode: &grading_mode_v,
                    max_points: max_points_v,
                    lesson_id: None,
                    allow_late: *allow_late.read(),
                    lock_on_submit: *lock_on_submit.read(),
                    accepts_text: *accepts_text.read(),
                    accepts_files: *accepts_files.read(),
                    release_mode: &release_mode_v,
                    late_penalty_percent: *late_penalty_percent.read(),
                    max_resubmissions: *max_resubmissions.read(),
                    due_at: due_at_v.as_deref(),
                };
                match api::create_assignment(&api, &course_id, &body).await {
                    Ok(a) => {
                        toast.push(
                            ToastLevel::Success,
                            "Assignment saved",
                            "Draft created. Opening the assignment…",
                        );
                        #[cfg(target_arch = "wasm32")]
                        if let Some(win) = web_sys::window() {
                            let _ = win
                                .location()
                                .set_href(&format!("/courses/{course_slug}/assignments/{}", a.id));
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        let _ = (a, course_slug);
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        toast.push(ToastLevel::Danger, "Save failed", msg.clone());
                        error.set(Some(msg));
                    }
                }
            }
            saving.set(false);
        });
    };

    let grading_mode_options = vec![
        SelectOption {
            value: "numeric".into(),
            label: "Numeric".into(),
        },
        SelectOption {
            value: "pass_fail".into(),
            label: "Pass/fail".into(),
        },
    ];

    // Editor heading: tells the teacher at a glance whether they're creating
    // a new assignment or editing an existing one.
    let editor_heading = if initial.is_some() {
        "Edit assignment"
    } else {
        "New assignment"
    };

    rsx! {
        form { class: "assignment-editor motion-page", onsubmit: move |e| e.prevent_default(),
            h2 { class: "assignment-editor__title", "{editor_heading}" }
            Field { label: "Title".to_string(),
                Input {
                    value: title.read().clone(),
                    input_type: "text".to_string(),
                    on_input: move |v| title.set(v),
                }
            }

            Field {
                label: "Instructions (markdown)".to_string(),
                for_id: "assignment-instructions".to_string(),
                helper: "Use plain markdown for headings, lists, and links.".to_string(),
                textarea {
                    id: "assignment-instructions",
                    name: "instructions",
                    "aria-describedby": "assignment-instructions-description",
                    class: "ds-input",
                    rows: 6,
                    value: "{instructions}",
                    oninput: move |e| instructions.set(e.value()),
                }
            }

            fieldset { class: "assignment-editor__fieldset",
                legend { "Grading" }
                Field { label: "Mode".to_string(),
                    Select {
                        value: grading_mode.read().clone(),
                        options: grading_mode_options.clone(),
                        on_change: move |v| grading_mode.set(v),
                    }
                }
                if grading_mode.read().as_str() == "numeric" {
                    Field { label: "Max points".to_string(),
                        Input {
                            value: format!("{}", max_points.read()),
                            input_type: "number".to_string(),
                            on_input: move |v: String| {
                                if let Ok(n) = v.parse() { max_points.set(n); }
                            },
                        }
                    }
                }
            }

            fieldset { class: "assignment-editor__fieldset",
                legend { "Submission types" }
                Switch {
                    checked: *accepts_text.read(),
                    label: "Text".to_string(),
                    on_change: move |v| accepts_text.set(v),
                }
                Switch {
                    checked: *accepts_files.read(),
                    label: "Files".to_string(),
                    on_change: move |v| accepts_files.set(v),
                }
            }

            fieldset { class: "assignment-editor__fieldset",
                legend { "Policies" }
                Switch {
                    checked: *allow_late.read(),
                    label: "Allow late submissions".to_string(),
                    on_change: move |v| allow_late.set(v),
                }
                Switch {
                    checked: *lock_on_submit.read(),
                    label: "Lock submission once submitted".to_string(),
                    on_change: move |v| lock_on_submit.set(v),
                }
                Field {
                    label: "Late penalty (%)".to_string(),
                    helper: "Percentage docked from numeric grades on late submissions. 0 disables it.".to_string(),
                    Input {
                        value: format!("{}", late_penalty_percent.read()),
                        input_type: "number".to_string(),
                        on_input: move |v: String| {
                            if let Ok(n) = v.parse::<i32>() {
                                late_penalty_percent.set(n.clamp(0, 100));
                            }
                        },
                    }
                }
                Field {
                    label: "Max resubmissions".to_string(),
                    helper: "Times a student may resubmit after a return. 0 means no resubmission.".to_string(),
                    Input {
                        value: format!("{}", max_resubmissions.read()),
                        input_type: "number".to_string(),
                        on_input: move |v: String| {
                            if let Ok(n) = v.parse::<i32>() {
                                max_resubmissions.set(n.max(0));
                            }
                        },
                    }
                }
                div { class: "assignment-editor__radio-group",
                    label { class: "assignment-editor__radio",
                        Radio {
                            checked: release_mode.read().as_str() == "instant",
                            name: "release_mode".to_string(),
                            value: "instant".to_string(),
                            on_change: move |v| release_mode.set(v),
                        }
                        span { "Release grades instantly" }
                    }
                    label { class: "assignment-editor__radio",
                        Radio {
                            checked: release_mode.read().as_str() == "manual",
                            name: "release_mode".to_string(),
                            value: "manual".to_string(),
                            on_change: move |v| release_mode.set(v),
                        }
                        span { "Hold grades for manual release" }
                    }
                }
            }

            fieldset { class: "assignment-editor__fieldset",
                legend { "Schedule" }
                Field {
                    label: "Due date".to_string(),
                    helper: "Optional. Leave blank for no deadline.".to_string(),
                    DateTimePicker {
                        value: due_at.read().clone(),
                        disabled: *saving.read(),
                        on_change: move |v: String| due_at.set(v),
                    }
                }
            }

            fieldset { class: "assignment-editor__fieldset",
                legend { "Reference attachments" }
                if let Some(assignment_id) = existing_assignment_id.clone() {
                    FilePicker {
                        purpose: "attachment".to_string(),
                        linked_entity_type: "assignment_attachment".to_string(),
                        linked_entity_id: assignment_id.clone(),
                        allowed_types: validation::ATTACHMENT_TYPES.iter().map(|s| s.to_string()).collect(),
                        max_size_bytes: validation::ATTACHMENT_MAX,
                        on_uploaded: {
                            let api = api.clone();
                            move |asset_id: String| {
                                let api = api.clone();
                                let assignment_id = assignment_id.clone();
                                spawn(async move {
                                    let mut candidate_ids = attachment_asset_ids.read().clone();
                                    if !candidate_ids.iter().any(|id| id == &asset_id) {
                                        candidate_ids.push(asset_id);
                                    }

                                    let body = PatchAssignmentBody {
                                        attachment_asset_ids: Some(candidate_ids.clone()),
                                    };
                                    match api::patch_assignment(&api, &assignment_id, &body).await {
                                        Ok(_) => {
                                            attachment_asset_ids.set(candidate_ids);
                                            error.set(None);
                                        }
                                        Err(e) => error.set(Some(format!("{e}"))),
                                    }
                                });
                            }
                        },
                        button_label: "Attach reference file".to_string(),
                    }
                } else {
                    p { class: "muted", "Save the assignment draft before attaching reference files." }
                }
                if !attachment_asset_ids.read().is_empty() {
                    ul { class: "assignment-editor__attachments",
                        for asset_id in attachment_asset_ids.read().iter() {
                            li { key: "{asset_id}", "{asset_id}" }
                        }
                    }
                }
            }

            // Rubric builder. Only meaningful for a saved, numeric assignment:
            // the per-criterion scores sum to the numeric grade. Drafts (no id)
            // and pass/fail assignments prompt the teacher to save/switch first.
            // Wrapped in a `{}` Rust-expression block so the inner match/rsx!
            // branches are plain Rust (not rsx-native if-children).
            {
            if let Some(assignment_id) = existing_assignment_id.clone() {
                if grading_mode.read().as_str() == "numeric" {
                    match &*rubric_res.read_unchecked() {
                        Some(Ok(maybe_rubric)) => rsx! {
                            crate::rubric_editor::RubricEditor {
                                api: api.clone(),
                                assignment_id: assignment_id.clone(),
                                initial: maybe_rubric.clone(),
                            }
                        },
                        Some(Err(_)) => rsx! {
                            fieldset { class: "assignment-editor__fieldset",
                                legend { "Rubric (optional)" }
                                p { class: "muted", "Couldn't load the rubric. Reload to try again." }
                            }
                        },
                        None => rsx! {
                            fieldset { class: "assignment-editor__fieldset",
                                legend { "Rubric (optional)" }
                                p { class: "muted", "Loading rubric…" }
                            }
                        },
                    }
                } else {
                    rsx! {
                        fieldset { class: "assignment-editor__fieldset",
                            legend { "Rubric (optional)" }
                            p { class: "muted", "Rubrics apply to numeric assignments. Switch the grading mode to Numeric to add one." }
                        }
                    }
                }
            } else {
                rsx! {
                    fieldset { class: "assignment-editor__fieldset",
                        legend { "Rubric (optional)" }
                        p { class: "muted", "Save the assignment draft before building a rubric." }
                    }
                }
            }
            }

            FormError { message: error.read().clone() }

            div { class: "assignment-editor__actions",
                Button {
                    label: if *saving.read() { "Saving…".to_string() } else { "Save draft".to_string() },
                    variant: ButtonVariant::Primary,
                    disabled: *saving.read(),
                    on_click: on_save,
                }
                if let Some(aid) = existing_assignment_id.as_ref() {
                    {
                        let aid = aid.clone();
                        let aid_unpublish = aid.clone();
                        let aid_publish = aid.clone();
                        let delete_api = api.clone();
                        let unpublish_api = api.clone();
                        let publish_api = api.clone();
                        let course_slug_for_delete = props.course_slug.clone();
                        let status = initial
                            .as_ref()
                            .map(|a| a.status.clone())
                            .unwrap_or_default();
                        let is_published = status == "published";
                        let is_draft = status == "draft";
                        rsx! {
                            if is_draft {
                                Button {
                                    label: "Publish".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    disabled: *saving.read(),
                                    on_click: move |_| {
                                        let aid = aid_publish.clone();
                                        let publish_api = publish_api.clone();
                                        spawn(async move {
                                            saving.set(true);
                                            match api::publish_assignment(&publish_api, &aid).await {
                                                Ok(_) => {
                                                    toast.push(
                                                        ToastLevel::Success,
                                                        "Assignment published",
                                                        "Students can now see this assignment.",
                                                    );
                                                    // Reload so the editor reflects the
                                                    // new published status (mirrors Unpublish).
                                                    #[cfg(target_arch = "wasm32")]
                                                    if let Some(win) = web_sys::window() {
                                                        let _ = win.location().reload();
                                                    }
                                                }
                                                Err(e) => {
                                                    let msg = format!("{e}");
                                                    toast.push(ToastLevel::Danger, "Publish failed", msg.clone());
                                                    error.set(Some(msg));
                                                }
                                            }
                                            saving.set(false);
                                        });
                                    },
                                }
                            }
                            if is_published {
                                Button {
                                    label: "Unpublish".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    disabled: *saving.read(),
                                    on_click: move |_| {
                                        let aid = aid_unpublish.clone();
                                        let unpublish_api = unpublish_api.clone();
                                        spawn(async move {
                                            saving.set(true);
                                            match api::unpublish_assignment(&unpublish_api, &aid).await {
                                                Ok(_) => {
                                                    toast.push(
                                                        ToastLevel::Success,
                                                        "Assignment unpublished",
                                                        "Students can no longer see this assignment.",
                                                    );
                                                    // Reload the editor with the new
                                                    // status by navigating back to the
                                                    // detail page (which re-fetches).
                                                    #[cfg(target_arch = "wasm32")]
                                                    if let Some(win) = web_sys::window() {
                                                        let _ = win.location().reload();
                                                    }
                                                }
                                                Err(e) => {
                                                    let msg = format!("{e}");
                                                    toast.push(ToastLevel::Danger, "Unpublish failed", msg.clone());
                                                    error.set(Some(msg));
                                                }
                                            }
                                            saving.set(false);
                                        });
                                    },
                                }
                            }
                            Button {
                                label: "Delete".to_string(),
                                variant: ButtonVariant::Danger,
                                disabled: *saving.read(),
                                on_click: move |_| {
                                    let aid = aid.clone();
                                    let delete_api = delete_api.clone();
                                    let course_slug = course_slug_for_delete.clone();
                                    spawn(async move {
                                        saving.set(true);
                                        match api::delete_assignment(&delete_api, &aid).await {
                                            Ok(_) => {
                                                toast.push(
                                                    ToastLevel::Success,
                                                    "Assignment deleted",
                                                    "The assignment was removed.",
                                                );
                                                #[cfg(target_arch = "wasm32")]
                                                if let Some(win) = web_sys::window() {
                                                    let _ = win.location().set_href(
                                                        &format!("/courses/{course_slug}/assignments"),
                                                    );
                                                }
                                                #[cfg(not(target_arch = "wasm32"))]
                                                let _ = course_slug;
                                            }
                                            Err(e) => {
                                                let msg = format!("{e}");
                                                toast.push(ToastLevel::Danger, "Delete failed", msg.clone());
                                                error.set(Some(msg));
                                            }
                                        }
                                        saving.set(false);
                                    });
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_api() -> ApiContext {
        ApiContext {
            base_url: "http://localhost:8080".into(),
            id_token: String::new(),
        }
    }

    fn assignment() -> AssignmentDto {
        AssignmentDto {
            id: "assignment-1".into(),
            course_id: "course-1".into(),
            lesson_id: None,
            title: "Essay".into(),
            instructions_md: "Write two pages.".into(),
            grading_mode: "numeric".into(),
            max_points: Some(100),
            allow_late: true,
            lock_on_submit: false,
            accepts_text: true,
            accepts_files: true,
            release_mode: "instant".into(),
            late_penalty_percent: 0,
            max_resubmissions: 0,
            attachment_asset_ids: vec!["asset-1".into()],
            due_at: None,
            status: "draft".into(),
            published_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn render_with_api_context(initial: Option<AssignmentDto>) -> String {
        use design_system::ToastQueue;

        let mut dom = VirtualDom::new_with_props(
            move |props: AssignmentEditorProps| {
                use_context_provider::<Signal<ApiContext>>(|| Signal::new(fake_api()));
                use_context_provider::<Signal<ToastQueue>>(|| Signal::new(ToastQueue::new()));
                rsx! { AssignmentEditor {
                    api: props.api.clone(),
                    course_slug: props.course_slug.clone(),
                    course_id: props.course_id.clone(),
                    initial: props.initial.clone(),
                } }
            },
            AssignmentEditorProps {
                api: fake_api(),
                course_slug: "math".into(),
                course_id: "course-1".into(),
                initial,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn saved_assignment_renders_reference_attachment_picker_and_ids() {
        let html = render_with_api_context(Some(assignment()));

        assert!(html.contains("Reference attachments"), "got: {html}");
        assert!(html.contains("ds-file-picker"), "got: {html}");
        assert!(html.contains("asset-1"), "got: {html}");
    }

    #[test]
    fn new_assignment_prompts_to_save_before_attaching_reference_files() {
        let html = render_with_api_context(None);

        assert!(
            html.contains("Save the assignment draft before attaching reference files."),
            "got: {html}"
        );
    }

    #[test]
    fn editor_heading_reflects_create_vs_edit() {
        let create_html = render_with_api_context(None);
        assert!(create_html.contains("New assignment"), "got: {create_html}");
        assert!(
            !create_html.contains("Edit assignment"),
            "got: {create_html}"
        );

        let edit_html = render_with_api_context(Some(assignment()));
        assert!(edit_html.contains("Edit assignment"), "got: {edit_html}");
        assert!(!edit_html.contains("New assignment"), "got: {edit_html}");
    }

    #[test]
    fn draft_assignment_shows_publish_button() {
        let html = render_with_api_context(Some(assignment()));
        assert!(html.contains(">Publish</button>"), "got: {html}");
        // Drafts must not offer Unpublish.
        assert!(!html.contains(">Unpublish</button>"), "got: {html}");
    }

    #[test]
    fn published_assignment_shows_unpublish_not_publish() {
        let published = AssignmentDto {
            status: "published".into(),
            ..assignment()
        };
        let html = render_with_api_context(Some(published));
        assert!(html.contains(">Unpublish</button>"), "got: {html}");
        assert!(!html.contains(">Publish</button>"), "got: {html}");
    }

    #[test]
    fn editor_renders_due_date_picker() {
        let html = render_with_api_context(Some(assignment()));
        assert!(html.contains("Due date"), "got: {html}");
        assert!(html.contains("ds-datetime"), "got: {html}");
    }

    #[test]
    fn editor_renders_late_penalty_and_resubmission_inputs() {
        let html = render_with_api_context(Some(assignment()));
        assert!(html.contains("Late penalty (%)"), "got: {html}");
        assert!(html.contains("Max resubmissions"), "got: {html}");
    }

    #[test]
    fn editor_prefills_policy_values_from_initial() {
        let with_policy = AssignmentDto {
            late_penalty_percent: 25,
            max_resubmissions: 3,
            ..assignment()
        };
        let html = render_with_api_context(Some(with_policy));
        // The number inputs render their current value as a `value="..."` attr.
        assert!(html.contains("value=\"25\""), "got: {html}");
        assert!(html.contains("value=\"3\""), "got: {html}");
    }

    #[test]
    fn datetime_local_to_rfc3339_adds_seconds_and_zone() {
        assert_eq!(
            datetime_local_to_rfc3339("2026-05-30T14:30").as_deref(),
            Some("2026-05-30T14:30:00Z")
        );
        // Seconds already present.
        assert_eq!(
            datetime_local_to_rfc3339("2026-05-30T14:30:45").as_deref(),
            Some("2026-05-30T14:30:45Z")
        );
        // Already zoned → untouched.
        assert_eq!(
            datetime_local_to_rfc3339("2026-05-30T14:30:00Z").as_deref(),
            Some("2026-05-30T14:30:00Z")
        );
        // Empty → cleared.
        assert_eq!(datetime_local_to_rfc3339("   "), None);
        assert_eq!(datetime_local_to_rfc3339(""), None);
    }

    #[test]
    fn rfc3339_to_datetime_local_trims_to_minutes() {
        assert_eq!(
            rfc3339_to_datetime_local("2026-05-30T14:30:00Z"),
            "2026-05-30T14:30"
        );
        assert_eq!(
            rfc3339_to_datetime_local("2026-05-30T14:30Z"),
            "2026-05-30T14:30"
        );
    }
}
