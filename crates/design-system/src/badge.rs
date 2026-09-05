// crates/design-system/src/badge.rs
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Primary,
    Info,
    Success,
    Warning,
    Danger,
    Live,
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum BadgeSize {
    Sm,
    #[default]
    Md,
}

#[derive(Props, Clone, PartialEq)]
pub struct BadgeProps {
    pub label: String,
    #[props(default = BadgeTone::Neutral)]
    pub tone: BadgeTone,
    #[props(default)]
    pub size: BadgeSize,
}

#[component]
pub fn Badge(props: BadgeProps) -> Element {
    let tone_class = match props.tone {
        BadgeTone::Neutral => "badge-neutral",
        BadgeTone::Primary => "badge-primary",
        BadgeTone::Info => "badge-info",
        BadgeTone::Success => "badge-success",
        BadgeTone::Warning => "badge-warning",
        BadgeTone::Danger => "badge-danger",
        BadgeTone::Live => "badge-live",
        BadgeTone::Premium => "badge-premium",
    };
    let size_class = match props.size {
        BadgeSize::Sm => "badge-sm",
        BadgeSize::Md => "badge-md",
    };
    let class = format!("badge {tone_class} {size_class}");
    let is_live = matches!(props.tone, BadgeTone::Live);
    rsx! {
        span { class: "{class}",
            if is_live {
                span { class: "badge-pulse", "aria-hidden": "true" }
            }
            "{props.label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_live_tone_renders_class() {
        fn app() -> Element {
            rsx! {
                Badge {
                    label: "LIVE".to_string(),
                    tone: BadgeTone::Live,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("badge-live"),
            "badge-live class missing: {html}"
        );
    }

    #[test]
    fn badge_premium_renders_class() {
        fn app() -> Element {
            rsx! { Badge { label: "Pro".to_string(), tone: BadgeTone::Premium } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("badge-premium"),
            "premium class missing: {html}"
        );
    }

    #[test]
    fn badge_size_sm_renders_class() {
        fn app() -> Element {
            rsx! { Badge { label: "x".to_string(), size: BadgeSize::Sm } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("badge-sm"), "sm class missing: {html}");
    }

    #[test]
    fn badge_live_renders_pulse_dot() {
        fn app() -> Element {
            rsx! { Badge { label: "LIVE".to_string(), tone: BadgeTone::Live } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("badge-pulse"), "pulse dot missing: {html}");
    }
}
