// crates/features-courses/src/live_now_banner.rs
//! Banner that students see on a course page when a live session is in
//! progress. Driven by `use_active_session_poll`. Hidden from admins —
//! their `StartNowButton` already reflects the conflict state.

use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct LiveNowBannerProps {
    pub title: String,
    pub on_join: EventHandler<()>,
}

#[component]
pub fn LiveNowBanner(props: LiveNowBannerProps) -> Element {
    rsx! {
        div { class: "live-now-banner motion-page",
            role: "status",
            aria_live: "polite",
            aria_label: "Live session notification",
            span { class: "live-now-banner-dot", aria_hidden: "true" }
            span { class: "live-now-banner-label", "Live now" }
            span { class: "live-now-banner-title", "{props.title}" }
            Button {
                label: "Join".to_string(),
                variant: ButtonVariant::Primary,
                button_type: "button".to_string(),
                on_click: move |_| props.on_join.call(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_title_and_join_button() {
        fn app() -> Element {
            rsx! {
                LiveNowBanner {
                    title: "Quick session — May 17, 2:32 PM UTC".to_string(),
                    on_join: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Live now"),
            "missing 'Live now' label: {html}"
        );
        assert!(html.contains("Quick session"), "missing title: {html}");
        assert!(
            html.contains(">Join</button>"),
            "missing Join button: {html}"
        );
    }
}
