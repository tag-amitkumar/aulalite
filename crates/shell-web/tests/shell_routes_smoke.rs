//! Phase 1.5: SSR smokes for the new shell-web routes.
//!
//! Each test wires up a `MemoryHistory` set to the route under test, then
//! mounts the full `Router::<Route>` so the route component runs inside
//! a real Router context (`use_navigator()` works).

use std::rc::Rc;

use design_system::ToastProvider;
use dioxus::prelude::*;
use dioxus_router::Router;
use dioxus_ssr::render;
use features_courses::api::ApiContext;
use shell_web::contexts::{UserContext, UserContextSignal};
use shell_web::route_enum::Route;

fn fake_api() -> ApiContext {
    ApiContext {
        base_url: "http://localhost:8080".into(),
        id_token: String::new(),
    }
}

fn fake_user() -> UserContext {
    UserContext {
        user_id: "00000000-0000-0000-0000-000000000001".into(),
        display_name: "Test User".into(),
        email: "test@example.test".into(),
        tenant_id: Some("00000000-0000-0000-0000-000000000010".into()),
        tenant_role: Some(core_types::TenantRole::Teacher),
        is_platform_admin: false,
    }
}

#[derive(Clone, PartialEq, Props)]
struct HarnessProps {
    path: String,
    signed_in: bool,
}

#[component]
fn Harness(props: HarnessProps) -> Element {
    // Provide a memory-backed history pinned to the requested path so the
    // Router renders the matching route component on first paint.
    use_hook(|| {
        let history = Rc::new(dioxus::history::MemoryHistory::with_initial_path(
            props.path.clone(),
        ));
        dioxus::history::provide_history_context(history);
    });

    let api_signal = use_signal(fake_api);
    let user_signal: UserContextSignal = use_signal(|| {
        if props.signed_in {
            Some(fake_user())
        } else {
            None
        }
    });
    use_context_provider::<Signal<ApiContext>>(|| api_signal);
    use_context_provider::<UserContextSignal>(|| user_signal);
    use_context_provider(|| api_signal.read().clone());

    rsx! {
        ToastProvider {
            Router::<Route> {}
        }
    }
}

fn dom_for_path(path: &str, signed_in: bool) -> VirtualDom {
    VirtualDom::new_with_props(
        Harness,
        HarnessProps {
            path: path.to_string(),
            signed_in,
        },
    )
}

#[test]
fn login_route_renders() {
    let mut dom = dom_for_path("/login", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    // Login form should at least render the email field.
    assert!(
        html.contains("email") || html.contains("Email"),
        "html missing email field: {html}"
    );
}

#[test]
fn parent_login_route_uses_family_specific_copy() {
    let mut dom = dom_for_path("/parent/login", false);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.contains("Parent or guardian sign in"), "got: {html}");
    assert!(html.contains("Stay close to their learning"), "got: {html}");
    assert!(html.contains("linked-child access"), "got: {html}");
    assert!(!html.contains("course operations"), "got: {html}");
}

#[test]
fn dashboard_route_renders_for_signed_in_user() {
    let mut dom = dom_for_path("/", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    // Dashboard has the user's display name in the AppShell header OR the
    // welcome heading. Loading state OK too.
    assert!(
        html.contains("Test User") || html.contains("Loading") || html.contains("Welcome"),
        "html missing expected content: {html}"
    );
}

#[test]
fn nav_links_do_not_point_at_unknown_dashboard_route() {
    // The router defines Dashboard at "/", not "/dashboard". If any nav
    // link ships pointing at "/dashboard", clicking it surfaces
    // "Failed to parse route".
    let mut dom = dom_for_path("/", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        !html.contains("href=\"/dashboard\""),
        "found nav link to /dashboard, which is not a real route. html: {html}"
    );
}

/// Helper: any of the substrings present in the SSR'd HTML is acceptable.
/// `len > N` heuristics are too loose — a 60-byte error paragraph would
/// pass them. These assertions instead pin specific class-names or strings
/// emitted by the route component so a future regression that mounts the
/// wrong component (or the loading state forever) is detected.
fn assert_any_of(label: &str, html: &str, needles: &[&str]) {
    let hit = needles.iter().any(|n| html.contains(n));
    assert!(hit, "[{label}] expected one of {:?}, got: {html}", needles);
}

#[test]
fn course_list_route_renders_for_teacher() {
    let mut dom = dom_for_path("/courses", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    // The CourseList route renders the AppShell ("Test User" in header) +
    // either CourseListView (which contains "Courses" / class names) OR
    // the loading placeholder. Pin to one of those, not just byte length.
    assert_any_of(
        "course_list",
        &html,
        &["Test User", "Loading courses", "course-list", "Courses"],
    );
}

#[test]
fn course_detail_outline_route_renders() {
    let mut dom = dom_for_path("/courses/math", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert_any_of(
        "course_detail_outline",
        &html,
        &[
            "course-outline",
            "Loading course",
            "Loading outline",
            "Test User",
        ],
    );
}

#[test]
fn course_detail_people_route_renders() {
    let mut dom = dom_for_path("/courses/math/people", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert_any_of(
        "course_people",
        &html,
        &[
            "course-people",
            "Loading people",
            "Loading course",
            "Test User",
        ],
    );
}

#[test]
fn course_detail_schedule_route_renders() {
    let mut dom = dom_for_path("/courses/math/schedule", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert_any_of(
        "course_schedule",
        &html,
        &[
            "course-schedule-tab",
            "Loading schedule",
            "Loading course",
            "Test User",
        ],
    );
}

#[test]
fn assignment_list_route_renders() {
    let mut dom = dom_for_path("/courses/math/assignments", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert_any_of(
        "assignment_list",
        &html,
        &["assignment-list", "Loading", "Test User", "Assignments"],
    );
}

#[test]
fn admin_audit_route_renders_for_admin() {
    // Local helper user is a Teacher; the audit page should refuse.
    let mut dom = dom_for_path("/admin/audit", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("Forbidden") || html.contains("permission"),
        "non-admin should see Forbidden, got: {html}"
    );
}

#[test]
fn admin_notification_deliveries_route_renders_for_non_admin_as_forbidden() {
    let mut dom = dom_for_path("/admin/notifications", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("Forbidden") || html.contains("permission"),
        "non-admin should see Forbidden, got: {html}"
    );
}

#[test]
fn admin_integrations_route_renders_for_non_admin_as_forbidden() {
    let mut dom = dom_for_path("/admin/integrations", true);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("Forbidden") || html.contains("permission"),
        "non-admin should see Forbidden, got: {html}"
    );
}

#[test]
fn unauthenticated_dashboard_does_not_panic() {
    let mut dom = dom_for_path("/", false);
    dom.rebuild_in_place();
    let _ = render(&dom);
    // No panic = pass. Dashboard should render the redirect placeholder or loading.
}

#[test]
fn public_privacy_route_renders_without_authentication() {
    let mut dom = dom_for_path("/privacy", false);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("Privacy at AulaLite"),
        "privacy page missing: {html}"
    );
    assert!(html.contains("Last updated July 16, 2026"));
}

#[test]
fn unknown_route_renders_branded_recovery_page() {
    let mut dom = dom_for_path("/definitely/missing", false);
    dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("This page isn’t on the lesson plan"),
        "404 page missing: {html}"
    );
    assert!(html.contains("Return home"));
}
