// crates/features-courses/src/course_detail.rs
use crate::file_asset_image::FileAssetImage;
use design_system::{
    Badge, BadgeTone, Button, ButtonVariant, PageHeader, PageHeaderVariant, Tab, Tabs,
};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseDetailProps {
    pub course_title: String,
    pub course_status: String,
    pub course_cover_asset_id: Option<String>,
    pub can_admin: bool,
    pub active_tab: String,
    pub on_tab_change: EventHandler<String>,
    /// Additional actions to render in the PageHeader actions slot
    /// (e.g., StartNowButton for admins). Rendered between the status
    /// badge and the existing Edit button.
    #[props(default)]
    pub extra_actions: Option<Element>,
    /// Banner rendered below the PageHeader and above the tabs.
    /// Used for the student-facing LiveNowBanner.
    #[props(default)]
    pub banner: Option<Element>,
    pub children: Element,
}

#[component]
pub fn CourseDetail(props: CourseDetailProps) -> Element {
    let mut tabs: Vec<Tab> = vec![
        Tab {
            key: "outline".to_string(),
            label: "Outline".to_string(),
            disabled: false,
        },
        Tab {
            key: "assignments".to_string(),
            label: "Assignments".to_string(),
            disabled: false,
        },
        Tab {
            key: "announcements".to_string(),
            label: "Announcements".to_string(),
            disabled: false,
        },
        Tab {
            key: "discussions".to_string(),
            label: "Discussions".to_string(),
            disabled: false,
        },
        Tab {
            key: "scorm".to_string(),
            label: "SCORM".to_string(),
            disabled: false,
        },
        Tab {
            key: "syllabus".to_string(),
            label: "Syllabus".to_string(),
            disabled: false,
        },
        Tab {
            key: "quizzes".to_string(),
            label: "Quizzes".to_string(),
            disabled: false,
        },
        Tab {
            key: "leaderboard".to_string(),
            label: "Leaderboard".to_string(),
            disabled: false,
        },
        Tab {
            key: "certificates".to_string(),
            label: "Certificates".to_string(),
            disabled: false,
        },
        Tab {
            key: "flashcards".to_string(),
            label: "Flashcards".to_string(),
            disabled: false,
        },
        Tab {
            key: "schedule".to_string(),
            label: "Schedule".to_string(),
            disabled: false,
        },
        Tab {
            key: "recordings".to_string(),
            label: "Recordings".to_string(),
            disabled: false,
        },
        Tab {
            key: "analytics".to_string(),
            label: "Analytics".to_string(),
            disabled: false,
        },
    ];
    if props.can_admin {
        tabs.push(Tab {
            key: "people".to_string(),
            label: "People".to_string(),
            disabled: false,
        });
        tabs.push(Tab {
            key: "gradebook".to_string(),
            label: "Gradebook".to_string(),
            disabled: false,
        });
        tabs.push(Tab {
            key: "edit".to_string(),
            label: "Edit".to_string(),
            disabled: false,
        });
    }
    let on_change = props.on_tab_change;

    let status_tone = match props.course_status.as_str() {
        "draft" => BadgeTone::Neutral,
        "published" => BadgeTone::Success,
        "archived" => BadgeTone::Warning,
        _ => BadgeTone::Neutral,
    };
    let status_label = props.course_status.clone();
    let actions: Option<Element> = if props.can_admin {
        let on_change_edit = props.on_tab_change;
        let extras = props.extra_actions.clone();
        Some(rsx! {
            Badge { label: status_label, tone: status_tone }
            { extras }
            Button {
                label: "Edit".to_string(),
                variant: ButtonVariant::Ghost,
                button_type: "button".to_string(),
                on_click: move |_| on_change_edit.call("edit".to_string()),
            }
        })
    } else {
        let extras = props.extra_actions.clone();
        Some(rsx! {
            Badge { label: status_label, tone: status_tone }
            { extras }
        })
    };

    rsx! {
        div { class: "course-detail motion-page",
            if let Some(asset_id) = &props.course_cover_asset_id {
                div { class: "course-detail-banner",
                    FileAssetImage {
                        asset_id: asset_id.clone(),
                        alt: props.course_title.clone(),
                        class: Some("course-banner-img".to_string()),
                    }
                }
            } else {
                // No uploaded cover: show the same deterministic generated art as
                // the course card (was: no banner at all).
                div { class: "course-detail-banner",
                    design_system::CourseCoverArt {
                        seed: props.course_title.clone(),
                        title: props.course_title.clone(),
                        class: "course-gen-cover course-gen-cover--banner".to_string(),
                    }
                }
            }
            PageHeader {
                kicker: "Course".to_string(),
                title: props.course_title.clone(),
                variant: PageHeaderVariant::Hero,
                actions: actions,
            }
            { props.banner.clone() }
            Tabs { tabs: tabs, active: props.active_tab.clone(),
                on_change: move |k| on_change.call(k) }
            div { class: "course-detail-body", {props.children} }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn course_detail_renders_analytics_tab() {
        fn app() -> Element {
            rsx! {
                CourseDetail {
                    course_title: "Math".to_string(),
                    course_status: "draft".to_string(),
                    course_cover_asset_id: None::<String>,
                    can_admin: false,
                    active_tab: "outline".to_string(),
                    on_tab_change: move |_: String| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Analytics"), "expected Analytics tab: {html}");
        assert!(
            html.contains("Recordings"),
            "expected Recordings tab: {html}"
        );
    }
}
