// crates/shell-web/src/routes/search.rs
//
// Global search page. Available to every signed-in role (the backend
// `/v1/search` endpoint is authed and scopes visibility for the caller).
//
// Reads the initial query from the URL `?q=` on mount (parsing
// `window.location.search` on wasm; empty off-wasm). Binds a query signal to a
// search Input (prefilled) and, on submit (Enter or the Search button), calls
// `api::search` and renders the results. Follows the project-wide four-state
// UX: idle (empty-query prompt), loading, no-results (EmptyState), and results
// (a "Courses" section and an "Assignments" section).
use design_system::{Badge, BadgeTone, EmptyState, Input, PageHeader};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{self, AssignmentHitDto, CourseHitDto};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Default cap for each result section.
const SEARCH_LIMIT: u32 = 20;

/// Map an entity status string to a Badge tone. Pure so the result rows stay
/// SSR-testable.
fn status_tone(status: &str) -> BadgeTone {
    match status {
        "published" => BadgeTone::Success,
        "draft" => BadgeTone::Neutral,
        "archived" => BadgeTone::Warning,
        _ => BadgeTone::Neutral,
    }
}

/// Render the "Courses" results section. Each hit links to `/courses/{slug}`
/// with a status Badge. Pure/presentational so it's SSR-testable.
fn courses_section(courses: &[CourseHitDto]) -> Element {
    if courses.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "search-section",
            h2 { class: "search-section-title", "Courses" }
            ul { class: "search-result-list",
                for c in courses.iter() {
                    li { key: "{c.id}", class: "search-result-item",
                        a { class: "search-result-link", href: "/courses/{c.slug}",
                            span { class: "search-result-title", "{c.title}" }
                            Badge { label: c.status.clone(), tone: status_tone(&c.status) }
                        }
                    }
                }
            }
        }
    }
}

/// Render the "Assignments" results section. Each hit links to
/// `/courses/{course_slug}/assignments/{id}` with a status Badge.
/// Pure/presentational so it's SSR-testable.
fn assignments_section(assignments: &[AssignmentHitDto]) -> Element {
    if assignments.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "search-section",
            h2 { class: "search-section-title", "Assignments" }
            ul { class: "search-result-list",
                for a in assignments.iter() {
                    li { key: "{a.id}", class: "search-result-item",
                        a {
                            class: "search-result-link",
                            href: "/courses/{a.course_slug}/assignments/{a.id}",
                            span { class: "search-result-title", "{a.title}" }
                            Badge { label: a.status.clone(), tone: status_tone(&a.status) }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Search() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    // The query bound to the input (prefilled from the URL on first render).
    let mut query = use_signal(api::initial_search_query);
    // The query that was actually submitted (drives the fetch). Seeded from the
    // URL so a deep-link to /search?q=foo runs the search on mount.
    let mut submitted = use_signal(api::initial_search_query);

    let results = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            let q = submitted.read().trim().to_string();
            async move {
                if q.is_empty() {
                    // Idle: no query yet. Signalled as Ok(None) so the view can
                    // distinguish "nothing searched" from "no results".
                    Ok(None)
                } else {
                    api::search(&api, &q, SEARCH_LIMIT).await.map(Some)
                }
            }
        }
    });

    let submit = move |_| {
        let q = query.read().trim().to_string();
        submitted.set(q);
    };

    let content: Element = {
        let snap = results.read_unchecked();
        match snap.as_ref() {
            // Idle — no query submitted yet.
            Some(Ok(None)) => rsx! {
                EmptyState {
                    title: "Search your workspace".to_string(),
                    description: "Find courses and assignments by title.".to_string(),
                }
            },
            // Results resolved.
            Some(Ok(Some(resp))) => {
                if resp.courses.is_empty() && resp.assignments.is_empty() {
                    rsx! {
                        EmptyState {
                            title: "No results".to_string(),
                            description: "No courses or assignments matched your search.".to_string(),
                        }
                    }
                } else {
                    rsx! {
                        div { class: "search-results",
                            {courses_section(&resp.courses)}
                            {assignments_section(&resp.assignments)}
                        }
                    }
                }
            }
            // Error.
            Some(Err(e)) => rsx! {
                p { class: "error", "Search failed: {e}" }
            },
            // Loading.
            None => rsx! {
                p { "Searching…" }
            },
        }
    };

    let body = rsx! {
        div { class: "search-page",
            PageHeader {
                title: "Search".to_string(),
                kicker: "Workspace".to_string(),
                subtitle: "Find courses and assignments.".to_string(),
            }
            form {
                class: "search-form",
                role: "search",
                onsubmit: submit,
                Input {
                    value: query.read().clone(),
                    placeholder: "Search courses and assignments…".to_string(),
                    input_type: "search".to_string(),
                    name: "q".to_string(),
                    on_input: move |v: String| query.set(v),
                }
                button { class: "search-form-submit", r#type: "submit", "Search" }
            }
            { content }
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn courses_section_renders_links_and_badges() {
        fn app() -> Element {
            let courses = vec![
                CourseHitDto {
                    id: "c1".into(),
                    slug: "algebra".into(),
                    title: "Algebra I".into(),
                    status: "published".into(),
                },
                CourseHitDto {
                    id: "c2".into(),
                    slug: "geometry".into(),
                    title: "Geometry".into(),
                    status: "draft".into(),
                },
            ];
            courses_section(&courses)
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Courses"), "section title missing: {html}");
        assert!(
            html.contains("href=\"/courses/algebra\""),
            "course link missing: {html}"
        );
        assert!(
            html.contains("href=\"/courses/geometry\""),
            "course link missing: {html}"
        );
        assert!(html.contains("Algebra I"));
        // Status badges render.
        assert!(html.contains("badge"), "badge missing: {html}");
        assert!(html.contains("published"));
        assert!(html.contains("draft"));
    }

    #[test]
    fn assignments_section_renders_nested_links() {
        fn app() -> Element {
            let assignments = vec![AssignmentHitDto {
                id: "a1".into(),
                course_id: "c1".into(),
                course_slug: "algebra".into(),
                title: "Homework 1".into(),
                status: "published".into(),
            }];
            assignments_section(&assignments)
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Assignments"),
            "section title missing: {html}"
        );
        assert!(
            html.contains("href=\"/courses/algebra/assignments/a1\""),
            "assignment link missing: {html}"
        );
        assert!(html.contains("Homework 1"));
    }

    #[test]
    fn empty_sections_render_nothing() {
        fn app() -> Element {
            rsx! {
                div {
                    {courses_section(&[])}
                    {assignments_section(&[])}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("Courses"), "courses leaked: {html}");
        assert!(!html.contains("Assignments"), "assignments leaked: {html}");
        assert!(!html.contains("search-result-link"), "links leaked: {html}");
    }

    #[test]
    fn search_results_render_both_sections_together() {
        // Mirrors the loaded "results" branch of the page body.
        fn app() -> Element {
            let courses = vec![CourseHitDto {
                id: "c1".into(),
                slug: "algebra".into(),
                title: "Algebra I".into(),
                status: "published".into(),
            }];
            let assignments = vec![AssignmentHitDto {
                id: "a1".into(),
                course_id: "c1".into(),
                course_slug: "algebra".into(),
                title: "Homework 1".into(),
                status: "draft".into(),
            }];
            rsx! {
                div { class: "search-results",
                    {courses_section(&courses)}
                    {assignments_section(&assignments)}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Courses"));
        assert!(html.contains("Assignments"));
        assert!(html.contains("href=\"/courses/algebra\""));
        assert!(html.contains("href=\"/courses/algebra/assignments/a1\""));
    }
}
