// crates/shell-web/src/routes/dashboard.rs
use design_system::{Button, ButtonVariant, Card, PageHeader, SkeletonCard, SkeletonLine};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::dashboard::{Dashboard as DashboardView, EnrolledCourse};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
fn DashboardLoading() -> Element {
    rsx! {
        div {
            class: "dashboard-state dashboard-state--loading",
            role: "status",
            "aria-live": "polite",
            "aria-label": "Loading your dashboard",
            div { class: "dashboard-loading-hero",
                div { class: "dashboard-loading-copy",
                    SkeletonLine { width: "8rem".to_string(), height: "12px".to_string() }
                    SkeletonLine { width: "min(28rem, 82%)".to_string(), height: "38px".to_string() }
                    SkeletonLine { width: "min(36rem, 94%)".to_string(), height: "16px".to_string() }
                }
            }
            div { class: "dashboard-loading-metrics", "aria-hidden": "true",
                SkeletonCard { height: "112px".to_string() }
                SkeletonCard { height: "112px".to_string() }
                SkeletonCard { height: "112px".to_string() }
            }
            div { class: "dashboard-loading-panels", "aria-hidden": "true",
                SkeletonCard { height: "260px".to_string() }
                SkeletonCard { height: "260px".to_string() }
            }
            span { class: "sr-only", "Loading your courses and upcoming sessions." }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct DashboardFailureProps {
    on_retry: EventHandler<()>,
}

#[component]
fn DashboardFailure(props: DashboardFailureProps) -> Element {
    rsx! {
        section {
            class: "dashboard-state dashboard-state--error",
            role: "alert",
            "aria-labelledby": "dashboard-error-title",
            div { class: "dashboard-error-mark", "aria-hidden": "true", "!" }
            div { class: "dashboard-error-copy",
                p { class: "dashboard-error-eyebrow", "Temporary connection issue" }
                h2 { id: "dashboard-error-title", "We couldn’t refresh your dashboard" }
                p { "Your work is safe. Check your connection and try loading the latest courses and schedule again." }
            }
            Button {
                label: "Try again".to_string(),
                variant: ButtonVariant::Primary,
                on_click: move |_| props.on_retry.call(()),
            }
        }
    }
}

#[component]
pub fn Dashboard() -> Element {
    let nav = use_navigator();
    let mut signout = crate::routes::use_signout_action();
    let api = use_api();
    let user_ctx = use_user_context();

    if user_ctx.read().is_none() {
        return rsx! { crate::routes::landing::Landing {} };
    }

    let context = match user_ctx.read().as_ref() {
        Some(context) => context.clone(),
        None => return rsx! { p { "Loading…" } },
    };

    // Parents have a purpose-built, linked-child home. Keep `/` as a safe
    // compatibility entry, but never load learner/course dashboard data or
    // briefly expose generic learning chrome for a parent account.
    if context.tenant_role == Some(core_types::TenantRole::Parent) {
        nav.replace(Route::ParentHome {});
        return rsx! { p { "Opening your family overview…" } };
    }

    let user = ShellUser {
        display_name: context.display_name.clone(),
        email: context.email.clone(),
        tenant_role: context.tenant_role,
        is_platform_admin: context.is_platform_admin,
    };

    if context.is_platform_admin && context.tenant_id.is_none() {
        return rsx! {
            AppShell {
                user,
                on_signout: move |_| signout(),
                section { class: "page-stack motion-page platform-entry",
                    PageHeader {
                        kicker: "Elementors operations".to_string(),
                        title: format!("Welcome back, {}", context.display_name),
                        subtitle: "Manage organizations, platform health, and provisioning from the operator console.".to_string(),
                    }
                    Card {
                        div { class: "dashboard-upcoming-readout",
                            strong { "Platform" }
                            span { "Global operations context" }
                        }
                        p { class: "dashboard-upcoming-note",
                            "Tenant data stays isolated until your account has an active membership in that workspace."
                        }
                        a { class: "ds-button ds-button--primary", href: "/platform", "Open platform console" }
                    }
                }
            }
        };
    }

    let mut courses_resource = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_courses(&api).await }
        }
    });
    let mut schedule_resource = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_schedule(&api).await }
        }
    });
    // Lesson progress for the resume strip. Best-effort: a failure (e.g. a
    // teacher with no student enrollments) just hides the strip.
    let mut progress_resource = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_my_progress(&api).await }
        }
    });

    let display_name = user.display_name.clone();

    // Tri-state for both resources: loading | error | data. Errors must be
    // user-visible — silently rendering an empty dashboard would mask
    // outages.
    let courses_state: Result<Vec<EnrolledCourse>, Option<String>> = {
        let snap = courses_resource.read_unchecked();
        match snap.as_ref() {
            Some(Ok(cs)) => Ok(cs
                .iter()
                .map(|c| EnrolledCourse {
                    course_id: c.course_id.clone(),
                    slug: c.slug.clone(),
                    title: c.title.clone(),
                    status: c.status.clone(),
                    role: c.role.clone(),
                    next_session_at: c.next_session_at.clone(),
                })
                .collect()),
            Some(Err(e)) => Err(Some(format!("{e}"))),
            None => Err(None),
        }
    };
    let schedule_state: Result<usize, Option<String>> = {
        let snap = schedule_resource.read_unchecked();
        match snap.as_ref() {
            Some(Ok(s)) => Ok(s.len()),
            Some(Err(e)) => Err(Some(format!("{e}"))),
            None => Err(None),
        }
    };

    // Org-admins (and platform admins) with zero courses see the onboarding CTA
    // banner. It naturally disappears once a course exists — no backend
    // "onboarded" flag needed.
    let has_tenant_override = context.is_platform_admin && context.tenant_id.is_some();
    let is_admin = context.can_manage_organization();

    let my_progress: Vec<api::MyCourseProgressDto> = {
        let snap = progress_resource.read_unchecked();
        match snap.as_ref() {
            Some(Ok(p)) => p.clone(),
            _ => Vec::new(),
        }
    };
    let resume_strip: Element = rsx! {
        features_courses::gamify_panel::GamifyStrip {}
        features_courses::course_progress::ResumeLearningStrip {
            progress: my_progress,
            on_resume: move |(slug, lesson_id): (String, String)| {
                nav.push(Route::LessonPage { slug, lesson_id });
            },
        }
    };

    let dashboard_body: Element = match (&courses_state, &schedule_state) {
        (Ok(enrolled), Ok(upcoming_count)) => rsx! {
            {resume_strip}
            DashboardView {
                display_name: display_name.clone(),
                courses: enrolled.clone(),
                upcoming_count: *upcoming_count,
                show_setup_banner: is_admin && enrolled.is_empty(),
                tenant_role: user.tenant_role,
                is_platform_admin: has_tenant_override,
            }
        },
        (Err(Some(_)), _) | (_, Err(Some(_))) => rsx! {
            DashboardFailure {
                on_retry: move |_| {
                    courses_resource.restart();
                    schedule_resource.restart();
                    progress_resource.restart();
                },
            }
        },
        _ => rsx! { DashboardLoading {} },
    };

    rsx! {
        AppShell {
            user: user,
            on_signout: move |_| signout(),
            { dashboard_body }
        }
    }
}

#[cfg(test)]
mod ui_state_tests {
    use super::*;

    #[test]
    fn loading_state_is_accessible_and_structured() {
        let mut vdom = VirtualDom::new(|| rsx! { DashboardLoading {} });
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("role=\"status\""));
        assert!(html.contains("dashboard-loading-metrics"));
        assert!(html.contains("Loading your courses"));
    }

    #[test]
    fn failure_state_offers_a_retry_without_leaking_backend_details() {
        fn app() -> Element {
            rsx! { DashboardFailure { on_retry: move |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("Try again"));
        assert!(html.contains("Your work is safe"));
    }
}
