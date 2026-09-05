//! Draft-course setup checklist (teacher-flow uplift): a dismissible card on
//! the builder tab walking a teacher from empty course to first live class,
//! with live checkmarks computed from the course's actual content. Dismissal
//! persists per-course in localStorage — it's a helper, not state the backend
//! needs to know about.

use crate::api;
use design_system::Card;
use dioxus::prelude::*;

fn dismiss_key(course_id: &str) -> String {
    format!("aula.checklist-dismissed.{course_id}")
}

fn is_dismissed(course_id: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(&dismiss_key(course_id)).ok().flatten())
            .is_some()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = course_id;
        false
    }
}

fn persist_dismissed(course_id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item(&dismiss_key(course_id), "1");
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = course_id;
}

/// One checklist row. Pure so it's SSR-testable.
fn checklist_step(done: bool, label: &str, hint: &str, href: Option<&str>) -> Element {
    let state_class = if done {
        "course-checklist-step course-checklist-step--done"
    } else {
        "course-checklist-step"
    };
    rsx! {
        li { class: "{state_class}",
            span { class: "course-checklist-mark", aria_hidden: "true",
                if done { "✓" } else { "○" }
            }
            span { class: "course-checklist-label", "{label}" }
            if !done {
                if let Some(href) = href {
                    a { class: "course-checklist-link", href: "{href}", "{hint}" }
                } else {
                    span { class: "course-checklist-hint muted", "{hint}" }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseSetupChecklistProps {
    pub course_id: String,
    pub slug: String,
    /// Current course status ("draft" | "published" | "archived").
    pub course_status: String,
}

#[component]
pub fn CourseSetupChecklist(props: CourseSetupChecklistProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let mut dismissed = use_signal({
        let course_id = course_id.clone();
        move || is_dismissed(&course_id)
    });
    if *dismissed.read() {
        return rsx! {};
    }

    let outline = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&cx, &course_id).await }
        }
    });
    let quizzes = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::list_quizzes(&cx, &course_id).await }
        }
    });
    let sessions = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::list_course_sessions(&cx, &course_id).await }
        }
    });

    // Best-effort: unloaded/failed fetches count as zero — the checklist may
    // briefly under-report, never block.
    let (module_count, lesson_count) = {
        let snap = outline.read_unchecked();
        match snap.as_ref() {
            Some(Ok(mods)) => (
                mods.len(),
                mods.iter().map(|m| m.lessons.len()).sum::<usize>(),
            ),
            _ => (0, 0),
        }
    };
    let quiz_count = {
        let snap = quizzes.read_unchecked();
        match snap.as_ref() {
            Some(Ok(q)) => q.len(),
            _ => 0,
        }
    };
    let session_count = {
        let snap = sessions.read_unchecked();
        match snap.as_ref() {
            Some(Ok(s)) => s.len(),
            _ => 0,
        }
    };
    let published = props.course_status == "published";

    let slug = props.slug.clone();
    let quizzes_href = format!("/courses/{slug}/quizzes");
    let edit_href = format!("/courses/{slug}/edit");
    let schedule_href = format!("/courses/{slug}/schedule");

    let done_count = [
        module_count > 0,
        lesson_count > 0,
        quiz_count > 0,
        published,
        session_count > 0,
    ]
    .iter()
    .filter(|d| **d)
    .count();

    let on_dismiss = {
        let course_id = course_id.clone();
        move |_| {
            persist_dismissed(&course_id);
            dismissed.set(true);
        }
    };

    rsx! {
        Card {
            section { class: "course-checklist",
                header { class: "course-checklist-header",
                    div {
                        h3 { class: "course-checklist-title", "Set up your course" }
                        p { class: "course-checklist-progress muted", "{done_count} of 5 done" }
                    }
                    button {
                        class: "course-checklist-dismiss",
                        r#type: "button",
                        "aria-label": "Dismiss setup checklist",
                        onclick: on_dismiss,
                        "Dismiss"
                    }
                }
                ul { class: "course-checklist-steps",
                    { checklist_step(module_count > 0, "Add a module", "use “+ Module” below", None) }
                    { checklist_step(lesson_count > 0, "Add a lesson", "use “+ Add lesson” in a module", None) }
                    { checklist_step(quiz_count > 0, "Add a quiz", "Create one →", Some(quizzes_href.as_str())) }
                    { checklist_step(published, "Publish the course", "Change status →", Some(edit_href.as_str())) }
                    { checklist_step(session_count > 0, "Schedule your first live class", "Open the scheduler →", Some(schedule_href.as_str())) }
                }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn checklist_step_renders_done_and_pending_states() {
        fn app() -> Element {
            rsx! {
                ul {
                    { checklist_step(true, "Add a module", "hint", None) }
                    { checklist_step(false, "Add a quiz", "Create one →", Some("/courses/x/quizzes")) }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("course-checklist-step--done"), "got: {html}");
        assert!(html.contains("Add a quiz"));
        assert!(
            html.contains("/courses/x/quizzes"),
            "pending step links out: {html}"
        );
        // Done steps don't render their hint/link.
        assert!(
            !html.contains(">hint<"),
            "done step should hide hint: {html}"
        );
    }
}
