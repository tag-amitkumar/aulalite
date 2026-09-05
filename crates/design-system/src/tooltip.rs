use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct TooltipProps {
    pub label: String,
    pub children: Element,
    #[props(default)]
    pub side: TooltipSide,
}

#[derive(Clone, PartialEq, Default)]
pub enum TooltipSide {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[component]
pub fn Tooltip(props: TooltipProps) -> Element {
    let side_class = match props.side {
        TooltipSide::Top => "ds-tooltip ds-tooltip--top",
        TooltipSide::Bottom => "ds-tooltip ds-tooltip--bottom",
        TooltipSide::Left => "ds-tooltip ds-tooltip--left",
        TooltipSide::Right => "ds-tooltip ds-tooltip--right",
    };
    // Stable monotonic id so screen readers can associate the wrap (and any
    // focusable descendant inheriting it) with the tooltip text via
    // `aria-describedby`. Without this link the `role="tooltip"` span is
    // announced only by keyboard focus heuristics in some assistive tech.
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let id = format!("ds-tt-{}", N.fetch_add(1, Ordering::Relaxed));
    rsx! {
        span {
            class: "ds-tooltip-wrap",
            "aria-describedby": "{id}",
            {props.children}
            span {
                class: side_class,
                id: "{id}",
                role: "tooltip",
                "{props.label}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_renders_label_and_role() {
        fn app() -> Element {
            rsx! {
                Tooltip {
                    label: "Help".to_string(),
                    span { "hover me" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Help"), "label missing: {html}");
        assert!(html.contains("role=\"tooltip\""), "role missing: {html}");
        assert!(
            html.contains("ds-tooltip--top"),
            "default top class missing: {html}"
        );
    }

    #[test]
    fn tooltip_side_bottom_class_applied() {
        fn app() -> Element {
            rsx! {
                Tooltip {
                    label: "Bottom".to_string(),
                    side: TooltipSide::Bottom,
                    span { "x" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-tooltip--bottom"),
            "bottom class missing: {html}"
        );
    }
}
