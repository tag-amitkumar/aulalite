//! Phase 1c: SSR smokes — assignment + submission components mount without panic.

use design_system::ToastQueue;
use dioxus::prelude::*;
use features_courses::api::{ApiContext, SubmissionDto};

fn fake_api() -> ApiContext {
    ApiContext {
        base_url: "http://localhost:8080".into(),
        id_token: String::new(),
    }
}

#[test]
fn assignment_list_renders() {
    let mut vdom = VirtualDom::new_with_props(
        features_courses::assignment_list::AssignmentList,
        features_courses::assignment_list::AssignmentListProps {
            api: fake_api(),
            course_slug: "math".into(),
            course_id: "00000000-0000-0000-0000-000000000002".into(),
            can_author: true,
            can_view_drafts: true,
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Assignments"), "got: {html}");
    assert!(html.contains("assignment-shell"), "got: {html}");
}

#[test]
fn assignment_editor_renders() {
    use features_courses::assignment_editor::{AssignmentEditor, AssignmentEditorProps};
    let mut vdom = VirtualDom::new_with_props(
        |props: AssignmentEditorProps| {
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
            course_id: "00000000-0000-0000-0000-000000000002".into(),
            initial: None,
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Title"), "got: {html}");
    assert!(html.contains("Pass/fail"), "got: {html}");
}

#[test]
fn assignment_detail_renders_loading() {
    let mut vdom = VirtualDom::new_with_props(
        features_courses::assignment_detail::AssignmentDetail,
        features_courses::assignment_detail::AssignmentDetailProps {
            api: fake_api(),
            assignment_id: "00000000-0000-0000-0000-000000000001".into(),
            course_slug: "math".into(),
            current_user_id: "00000000-0000-0000-0000-000000000003".into(),
            can_author: false,
            can_grade: false,
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("system-state--loading"), "got: {html}");
}

#[test]
fn submission_view_hides_grade_when_unreleased() {
    let s = SubmissionDto {
        id: "00000000-0000-0000-0000-000000000010".into(),
        assignment_id: "00000000-0000-0000-0000-000000000001".into(),
        course_id: "00000000-0000-0000-0000-000000000002".into(),
        student_user_id: "00000000-0000-0000-0000-000000000003".into(),
        status: "graded".into(),
        text_answer: Some("answer".into()),
        attachment_asset_ids: vec![],
        submitted_at: Some("2026-05-09T00:00:00Z".into()),
        is_late: false,
        attempt_number: 1,
        applied_late_penalty_percent: None,
        numeric_grade: Some(80.0),
        letter_grade: None,
        passed: None,
        student_visible_feedback: Some("good".into()),
        graded_at: Some("2026-05-09T01:00:00Z".into()),
        released_at: None, // NOT released
        created_at: "2026-05-09T00:00:00Z".into(),
        updated_at: "2026-05-09T00:00:00Z".into(),
    };
    let mut vdom = VirtualDom::new_with_props(
        features_courses::submission_view::SubmissionView,
        features_courses::submission_view::SubmissionViewProps { submission: s },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        !html.contains("Numeric: 80"),
        "grade leaked when released_at NULL: {html}"
    );
    assert!(
        !html.contains("Feedback: good"),
        "feedback leaked when released_at NULL: {html}"
    );
}
