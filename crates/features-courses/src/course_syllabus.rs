// crates/features-courses/src/course_syllabus.rs
//! Course syllabus surfaces:
//!
//! * [`CourseSyllabusSettings`] — staff-only. Two `MarkdownEditor`s
//!   (syllabus + grading policy) that persist through `PATCH /v1/courses/:id`,
//!   plus a "Duplicate course" button that deep-copies the course into a new
//!   draft via `POST /v1/courses/:id/duplicate`. Mounted inside the course
//!   Edit tab (see shell-web `CourseEditTab`).
//! * [`CourseSyllabusView`] — read-only. Fetches `GET /v1/courses/:id/syllabus`
//!   and renders both markdown blocks for students. Mounted as the student-
//!   facing "Syllabus" tab.
//!
//! Markdown is rendered read-only through the shared safe markdown renderer.

use crate::api::{self, ApiContext};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, EmptyState, MarkdownEditor, SkeletonLine,
    ToastLevel,
};
use dioxus::prelude::*;

/// Render markdown read-only through the shared sanitizer.
fn render_markdown(body: &str, class: &str) -> Element {
    let out = crate::markdown::render_user_markdown_html(body);
    let cls = format!("lesson-md {class}");
    rsx! {
        div { class: "{cls}", dangerous_inner_html: "{out}" }
    }
}

// ---------------------------------------------------------------------------
// Staff: syllabus + grading-policy editor and the Duplicate button.
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct CourseSyllabusSettingsProps {
    pub course_id: String,
    /// Initial syllabus markdown (from the loaded CourseDto), if any.
    #[props(default)]
    pub initial_syllabus_md: Option<String>,
    /// Initial grading-policy markdown (from the loaded CourseDto), if any.
    #[props(default)]
    pub initial_grading_policy_md: Option<String>,
    /// Whether the catalog self-enrollment toggle is currently on.
    #[props(default)]
    pub initial_self_enrollment_enabled: bool,
    /// Called with the NEW course slug after a successful duplicate so the
    /// shell can navigate to the copy.
    pub on_duplicated: EventHandler<String>,
}

#[component]
pub fn CourseSyllabusSettings(props: CourseSyllabusSettingsProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();

    let mut syllabus = use_signal(|| props.initial_syllabus_md.clone().unwrap_or_default());
    let mut grading = use_signal(|| props.initial_grading_policy_md.clone().unwrap_or_default());
    let mut self_enroll = use_signal(|| props.initial_self_enrollment_enabled);
    let mut saving = use_signal(|| false);
    let mut duplicating = use_signal(|| false);
    let mut toast = use_toast_sender();

    let on_save = {
        let api = api.clone();
        let course_id = course_id.clone();
        move |_| {
            let api = api.clone();
            let course_id = course_id.clone();
            // Empty editor clears the field (Some(None)); otherwise set the value.
            let syllabus_value = syllabus.read().trim().to_string();
            let grading_value = grading.read().trim().to_string();
            saving.set(true);
            spawn(async move {
                let syllabus_opt: Option<&str> =
                    (!syllabus_value.is_empty()).then_some(syllabus_value.as_str());
                let grading_opt: Option<&str> =
                    (!grading_value.is_empty()).then_some(grading_value.as_str());
                let body = api::PatchCourseBody {
                    syllabus_md: Some(syllabus_opt),
                    grading_policy_md: Some(grading_opt),
                    self_enrollment_enabled: Some(*self_enroll.read()),
                    ..Default::default()
                };
                match api::patch_course(&api, &course_id, &body).await {
                    Ok(_) => {
                        toast.push(
                            ToastLevel::Success,
                            "Syllabus saved",
                            "Students can now read the updated syllabus.",
                        );
                    }
                    Err(e) => {
                        toast.push(ToastLevel::Danger, "Save failed", format!("{e}"));
                    }
                }
                saving.set(false);
            });
        }
    };

    let on_duplicate = {
        let api = api.clone();
        let course_id = course_id.clone();
        let on_duplicated = props.on_duplicated;
        move |_| {
            let api = api.clone();
            let course_id = course_id.clone();
            duplicating.set(true);
            spawn(async move {
                match api::duplicate_course(&api, &course_id).await {
                    Ok(new_course) => {
                        toast.push(
                            ToastLevel::Success,
                            "Course duplicated",
                            format!("Created \"{}\" as a draft.", new_course.title),
                        );
                        on_duplicated.call(new_course.slug);
                    }
                    Err(e) => {
                        toast.push(ToastLevel::Danger, "Duplicate failed", format!("{e}"));
                        duplicating.set(false);
                    }
                }
            });
        }
    };

    rsx! {
        section { class: "course-syllabus-settings",
            div { class: "course-syllabus-settings__head",
                h3 { "Syllabus & grading policy" }
                p { class: "muted",
                    "Markdown. Visible to enrolled students on the Syllabus tab."
                }
            }
            div { class: "course-syllabus-settings__field",
                label { class: "ds-label", "Syllabus" }
                MarkdownEditor {
                    value: syllabus.read().clone(),
                    disabled: *saving.read(),
                    on_change: move |v: String| syllabus.set(v),
                }
            }
            div { class: "course-syllabus-settings__field",
                label { class: "ds-label", "Grading policy" }
                MarkdownEditor {
                    value: grading.read().clone(),
                    disabled: *saving.read(),
                    on_change: move |v: String| grading.set(v),
                }
            }
            div { class: "course-syllabus-settings__field course-syllabus-settings__toggle",
                label {
                    input {
                        r#type: "checkbox",
                        checked: *self_enroll.read(),
                        onchange: move |e| self_enroll.set(e.checked()),
                    }
                    span { "Open for catalog self-enrollment" }
                }
                p { class: "muted",
                    "When on, any member of this workspace can find the published course in the Catalog and join without a code. Seat caps still apply."
                }
            }
            div { class: "course-syllabus-settings__actions",
                Button {
                    label: if *saving.read() { "Saving…".to_string() } else { "Save syllabus".to_string() },
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    disabled: *saving.read(),
                    on_click: on_save,
                }
            }
            div { class: "course-syllabus-settings__duplicate",
                h3 { "Duplicate this course" }
                p { class: "muted",
                    "Creates a new draft copy — modules, lessons, assignments and quizzes carry over. Enrollments, submissions and grades are not copied."
                }
                Button {
                    label: if *duplicating.read() { "Duplicating…".to_string() } else { "Duplicate course".to_string() },
                    variant: ButtonVariant::Secondary,
                    button_type: "button".to_string(),
                    disabled: *duplicating.read(),
                    on_click: on_duplicate,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read-only syllabus (students).
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct CourseSyllabusViewProps {
    pub course_id: String,
}

#[component]
pub fn CourseSyllabusView(props: CourseSyllabusViewProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();

    let syllabus = use_resource({
        let api: ApiContext = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_syllabus(&api, &course_id).await }
        }
    });

    rsx! {
        div { class: "course-syllabus motion-page",
            h2 { "Syllabus" }
            match &*syllabus.read_unchecked() {
                Some(Ok(s)) => {
                    let has_syllabus = s.syllabus_md.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);
                    let has_grading = s.grading_policy_md.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);
                    if !has_syllabus && !has_grading {
                        rsx! {
                            EmptyState {
                                title: "No syllabus yet".to_string(),
                                description: "Your teacher hasn't published a syllabus for this course yet.".to_string(),
                            }
                        }
                    } else {
                        rsx! {
                            if has_syllabus {
                                Card {
                                    div { class: "course-syllabus__block",
                                        h3 { "Overview" }
                                        { render_markdown(s.syllabus_md.as_deref().unwrap_or(""), "course-syllabus__body") }
                                    }
                                }
                            }
                            if has_grading {
                                Card {
                                    div { class: "course-syllabus__block",
                                        h3 { "Grading policy" }
                                        { render_markdown(s.grading_policy_md.as_deref().unwrap_or(""), "course-syllabus__body") }
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load the syllabus: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "60%".to_string() }
                        SkeletonLine { width: "90%".to_string() }
                        SkeletonLine { width: "75%".to_string() }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllabus_view_renders_heading() {
        fn app() -> Element {
            use design_system::ToastQueue;
            let api_signal = use_signal(|| api::ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            });
            use_context_provider::<Signal<api::ApiContext>>(|| api_signal);
            use_context_provider::<Signal<ToastQueue>>(|| Signal::new(ToastQueue::new()));
            rsx! {
                CourseSyllabusView { course_id: "c1".to_string() }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Syllabus"), "expected heading: {html}");
    }
}
