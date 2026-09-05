// crates/features-courses/src/course_list.rs
use design_system::{
    Badge, BadgeTone, Button, ButtonVariant, Card, CardList, EmptyState, PageHeader,
};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct CourseListItem {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub description: Option<String>,
    pub owner_user_id: String,
    pub cover_asset_id: Option<String>,
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseListProps {
    pub courses: Vec<CourseListItem>,
    pub can_create: bool,
    pub on_create_clicked: EventHandler<()>,
    /// Teachers see authoring vocabulary (draft/published/archived filter
    /// chips + status badges); students get a plain catalog without it.
    #[props(default)]
    pub is_teacher: bool,
}

#[component]
pub fn CourseList(props: CourseListProps) -> Element {
    let mut filter = use_signal(|| String::from("all"));
    let visible: Vec<CourseListItem> = props
        .courses
        .iter()
        .filter(|c| match filter.read().as_str() {
            "all" => true,
            x => c.status == x,
        })
        .cloned()
        .collect();

    let create_h = props.on_create_clicked;
    let actions = if props.can_create {
        Some(rsx! {
            Button {
                label: "New Course".to_string(),
                variant: ButtonVariant::Primary,
                on_click: move |_| create_h.call(()),
            }
        })
    } else {
        None
    };

    rsx! {
        div { class: "course-list-page motion-page",
            PageHeader {
                kicker: "Catalog".to_string(),
                title: "Courses".to_string(),
                actions: actions,
            }
            if props.is_teacher {
                div { class: "filter-chips",
                    for chip in &["all", "draft", "published", "archived"] {
                        {
                            let me = (*chip).to_string();
                            let active = *filter.read() == me;
                            let me_for_click = me.clone();
                            rsx! {
                                button {
                                    class: if active { "chip chip-active" } else { "chip" },
                                    onclick: move |_| filter.set(me_for_click.clone()),
                                    "{me}"
                                }
                            }
                        }
                    }
                }
            }
            if visible.is_empty() {
                EmptyState {
                    title: "No courses".to_string(),
                    description: if props.is_teacher {
                        "There aren't any courses matching this filter yet.".to_string()
                    } else {
                        "You're not enrolled in any courses yet.".to_string()
                    },
                    illustration: rsx! {
                        design_system::Illustration { kind: design_system::IllustrationKind::Courses }
                    },
                    cta: None,
                }
            } else {
                CardList {
                    for course in &visible {
                        li {
                            Card {
                                if let Some(asset_id) = &course.cover_asset_id {
                                    crate::file_asset_image::FileAssetImage {
                                        asset_id: asset_id.clone(),
                                        alt: course.title.clone(),
                                        class: Some("course-card-cover".to_string()),
                                    }
                                } else {
                                    design_system::CourseCoverArt {
                                        seed: course.slug.clone(),
                                        title: course.title.clone(),
                                        class: "course-card-cover-empty course-gen-cover".to_string(),
                                    }
                                }
                                h3 { a { href: "/courses/{course.slug}", "{course.title}" } }
                                if props.is_teacher {
                                    div { class: "card-row",
                                        Badge {
                                            label: course.status.clone(),
                                            tone: match course.status.as_str() {
                                                "draft" => BadgeTone::Neutral,
                                                "published" => BadgeTone::Success,
                                                "archived" => BadgeTone::Warning,
                                                _ => BadgeTone::Neutral,
                                            },
                                        }
                                    }
                                }
                                if let Some(desc) = &course.description {
                                    p { class: "course-desc", "{desc}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn create_button_hidden_when_not_allowed() {
        fn app() -> Element {
            rsx! {
                CourseList {
                    courses: vec![],
                    can_create: false,
                    on_create_clicked: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("New Course"));
    }

    #[test]
    fn page_header_renders_with_kicker_and_title() {
        fn app() -> Element {
            rsx! {
                CourseList {
                    courses: vec![],
                    can_create: true,
                    on_create_clicked: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-page-header"),
            "ds-page-header missing: {html}"
        );
        assert!(html.contains("Catalog"), "kicker missing: {html}");
        assert!(html.contains("Courses"), "title missing: {html}");
        assert!(html.contains("New Course"), "CTA missing: {html}");
    }

    #[test]
    fn card_list_renders_when_courses_present() {
        fn app() -> Element {
            rsx! {
                CourseList {
                    courses: vec![CourseListItem {
                        id: "1".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        description: Some("Intro to calculus".into()),
                        owner_user_id: "u".into(),
                        cover_asset_id: None,
                    }],
                    can_create: false,
                    on_create_clicked: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-card-list"),
            "ds-card-list missing: {html}"
        );
        assert!(html.contains("Calc 1"), "course title missing: {html}");
        assert!(
            html.contains("/courses/calc-1"),
            "course href missing: {html}"
        );
    }

    #[test]
    fn student_view_hides_status_chrome() {
        fn app() -> Element {
            rsx! {
                CourseList {
                    courses: vec![CourseListItem {
                        id: "1".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        description: None,
                        owner_user_id: "u".into(),
                        cover_asset_id: None,
                    }],
                    can_create: false,
                    on_create_clicked: |_| {},
                    is_teacher: false,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("filter-chips"),
            "status filter chips leaked to student view: {html}"
        );
        assert!(
            !html.contains("archived"),
            "authoring vocabulary leaked to student view: {html}"
        );
        assert!(
            !html.contains("published"),
            "status badge leaked to student view: {html}"
        );
        assert!(html.contains("Calc 1"), "course title missing: {html}");
    }

    #[test]
    fn teacher_view_keeps_status_chrome() {
        fn app() -> Element {
            rsx! {
                CourseList {
                    courses: vec![CourseListItem {
                        id: "1".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "draft".into(),
                        description: None,
                        owner_user_id: "u".into(),
                        cover_asset_id: None,
                    }],
                    can_create: true,
                    on_create_clicked: |_| {},
                    is_teacher: true,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("filter-chips"),
            "teacher filter chips missing: {html}"
        );
        assert!(html.contains("draft"), "status badge missing: {html}");
    }
}
