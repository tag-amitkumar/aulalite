// crates/features-courses/src/catalog_view.rs
//! Course catalog (`/catalog`): published courses in the workspace that have
//! self-enrollment enabled. Students join directly; seat caps are enforced by
//! the backend (a full workspace shows the same upgrade-prompt error as the
//! other enrollment paths).
use crate::api::{self, CatalogCourseDto};
use design_system::{Button, ButtonVariant, EmptyState, FormError, SkeletonCard};
use dioxus::prelude::*;

/// One catalog card. Pure/presentational so it is SSR-testable; navigation and
/// enrollment callbacks come from the parent.
pub fn catalog_card(
    course: &CatalogCourseDto,
    joining: bool,
    on_join: EventHandler<String>,
) -> Element {
    let key = course.course_id.clone();
    let title = course.title.clone();
    let owner = course.owner_name.clone();
    let description = course.description.clone();
    let enrolled = course.enrolled;
    let join_id = course.course_id.clone();
    rsx! {
        article { class: "catalog-card", key: "{key}",
            div { class: "catalog-card-body",
                h3 { class: "catalog-card-title", "{title}" }
                if let Some(owner) = owner {
                    p { class: "catalog-card-owner muted", "By {owner}" }
                }
                if let Some(desc) = description {
                    p { class: "catalog-card-desc", "{desc}" }
                }
            }
            div { class: "catalog-card-actions",
                if enrolled {
                    span { class: "catalog-card-enrolled", "Enrolled ✓" }
                } else if joining {
                    design_system::Loading { message: "Joining…".to_string() }
                } else {
                    Button {
                        label: "Join course".to_string(),
                        variant: ButtonVariant::Primary,
                        on_click: move |_| {
                            let id = join_id.clone();
                            on_join.call(id);
                        },
                    }
                }
            }
        }
    }
}

/// `/catalog` page body (the route supplies the shell + header). After a
/// successful join the component emits the course slug via `on_joined` so the
/// shell layer can navigate; an empty slug means "stay put".
#[component]
pub fn CatalogView(on_joined: EventHandler<String>) -> Element {
    let cx = api::use_api();
    let catalog = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::list_catalog(&cx).await }
        }
    });
    let mut joining: Signal<Option<String>> = use_signal(|| None);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let join = move |course_id: String| {
        let cx = cx.clone();
        joining.set(Some(course_id.clone()));
        spawn(async move {
            match api::catalog_self_enroll(&cx, &course_id).await {
                Ok(r) => {
                    joining.set(None);
                    // Resolve the slug for navigation.
                    match api::list_my_courses(&cx).await {
                        Ok(courses) => {
                            if let Some(c) = courses.iter().find(|c| c.course_id == r.course_id) {
                                on_joined.call(c.slug.clone());
                            } else {
                                on_joined.call(String::new());
                            }
                        }
                        Err(_) => on_joined.call(String::new()),
                    }
                }
                Err(e) => {
                    joining.set(None);
                    error.set(Some(format!("{e}")));
                }
            }
        });
    };

    let snap = catalog.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) => {
            let open: Vec<&CatalogCourseDto> = rows.iter().filter(|c| !c.enrolled).collect();
            let enrolled_count = rows.len() - open.len();
            if rows.is_empty() {
                rsx! {
                    EmptyState {
                        title: "The catalog is empty.".to_string(),
                        description: "When teachers open their courses for self-enrollment they appear here — no code needed.".to_string(),
                    }
                }
            } else {
                rsx! {
                    if enrolled_count > 0 {
                        p { class: "muted", "{enrolled_count} listed course(s) you already attend." }
                    }
                    FormError { message: error.read().clone() }
                    div { class: "catalog-grid",
                        for course in rows {
                            { catalog_card(
                                course,
                                joining.read().as_deref() == Some(course.course_id.as_str()),
                                EventHandler::new(join.clone()),
                            ) }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load the catalog: {e}" } },
        None => rsx! { SkeletonCard { height: "200px".to_string() } },
    };
    drop(snap);
    body
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn catalog_card_renders_join_or_enrolled_state() {
        fn joinable() -> Element {
            catalog_card(
                &CatalogCourseDto {
                    course_id: "co1".into(),
                    slug: "algebra".into(),
                    title: "Algebra I".into(),
                    description: Some("Numbers, but fun.".into()),
                    cover_asset_id: None,
                    owner_name: Some("Ada Lovelace".into()),
                    enrolled: false,
                },
                false,
                EventHandler::new(|_| {}),
            )
        }
        let mut vdom = VirtualDom::new(joinable);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Algebra I"), "title missing: {html}");
        assert!(html.contains("By Ada Lovelace"), "owner missing: {html}");
        assert!(html.contains("Join course"), "join button missing: {html}");

        fn enrolled() -> Element {
            catalog_card(
                &CatalogCourseDto {
                    course_id: "co2".into(),
                    slug: "calc".into(),
                    title: "Calculus".into(),
                    description: None,
                    cover_asset_id: None,
                    owner_name: None,
                    enrolled: true,
                },
                false,
                EventHandler::new(|_| {}),
            )
        }
        let mut vdom = VirtualDom::new(enrolled);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Enrolled ✓"),
            "enrolled badge missing: {html}"
        );
        assert!(!html.contains("Join course"), "no button expected: {html}");
    }
}
