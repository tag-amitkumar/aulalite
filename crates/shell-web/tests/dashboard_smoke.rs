// crates/shell-web/tests/dashboard_smoke.rs
//! SSR smoke test confirming the role-aware dashboard wires up correctly
//! for a teacher (no admin-only nav items leaked).

use dioxus::prelude::*;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::dashboard::Dashboard;

#[test]
fn renders_role_aware_dashboard_for_teacher() {
    fn app() -> Element {
        rsx! {
            AppShell {
                user: ShellUser {
                    display_name: "Teach".to_string(),
                    email: "t@x".to_string(),
                    tenant_role: Some(core_types::TenantRole::Teacher),
                    is_platform_admin: false,
                },
                on_signout: |_| {},
                Dashboard {
                    display_name: "Teach".to_string(),
                    courses: vec![],
                    upcoming_count: 0,
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("Welcome back, Teach"),
        "expected dashboard greeting in SSR output: {html}"
    );
    assert!(
        html.contains("app-shell-layout"),
        "expected app shell wrapper in SSR output: {html}"
    );
    assert!(
        html.contains("ds-page-header"),
        "expected editorial page header in SSR output: {html}"
    );
    assert!(
        html.contains("ui-metric-card"),
        "expected dashboard metric cards in SSR output: {html}"
    );
    assert!(
        html.contains("ds-page-header--hero"),
        "expected dashboard hero variant: {html}"
    );
    assert!(
        html.contains("motion-page"),
        "expected page animation class: {html}"
    );
    assert!(
        !html.contains("All Tenant Courses"),
        "teacher should not see admin-only nav items: {html}"
    );
    assert!(
        !html.contains("/me/schedule"),
        "shell must not link to the backend API path as a frontend route: {html}"
    );
    assert!(
        html.contains("/schedule") || !html.contains("My Schedule"),
        "frontend schedule links should use /schedule when present: {html}"
    );
}

#[test]
fn renders_student_schedule_nav_to_frontend_route() {
    fn app() -> Element {
        rsx! {
            AppShell {
                user: ShellUser {
                    display_name: "Student".to_string(),
                    email: "s@x".to_string(),
                    tenant_role: Some(core_types::TenantRole::Student),
                    is_platform_admin: false,
                },
                on_signout: |_| {},
                Dashboard {
                    display_name: "Student".to_string(),
                    courses: vec![],
                    upcoming_count: 0,
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("href=\"/schedule\""),
        "student schedule link should use /schedule: {html}"
    );
    assert!(
        !html.contains("href=\"/me/schedule\""),
        "student nav leaked API path: {html}"
    );
}

#[test]
fn renders_admin_nav_for_org_admin() {
    fn app() -> Element {
        rsx! {
            AppShell {
                user: ShellUser {
                    display_name: "Admin".to_string(),
                    email: "a@x".to_string(),
                    tenant_role: Some(core_types::TenantRole::OrgAdmin),
                    is_platform_admin: false,
                },
                on_signout: |_| {},
                Dashboard {
                    display_name: "Admin".to_string(),
                    courses: vec![],
                    upcoming_count: 0,
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("Workspace courses"),
        "org_admin should see admin nav: {html}"
    );
    assert!(
        !html.contains("href=\"/admin/billing\""),
        "org_admin should not see owner-only billing: {html}"
    );
}

#[test]
fn renders_owner_nav_with_billing() {
    fn app() -> Element {
        rsx! {
            AppShell {
                user: ShellUser {
                    display_name: "Owner".to_string(),
                    email: "owner@x".to_string(),
                    tenant_role: Some(core_types::TenantRole::OrgOwner),
                    is_platform_admin: false,
                },
                on_signout: |_| {},
                Dashboard {
                    display_name: "Owner".to_string(),
                    courses: vec![],
                    upcoming_count: 0,
                    tenant_role: Some(core_types::TenantRole::OrgOwner),
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("href=\"/admin/billing\""), "{html}");
    assert!(html.contains("Owner command center"), "{html}");
}
