use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonLineProps {
    /// Width as a CSS length (e.g. "100%", "12em"). Defaults to "100%".
    #[props(default = "100%".to_string())]
    pub width: String,
    /// Height as a CSS length. Defaults to "14px".
    #[props(default = "14px".to_string())]
    pub height: String,
}

#[component]
pub fn SkeletonLine(props: SkeletonLineProps) -> Element {
    rsx! {
        span {
            class: "ds-skeleton ds-skeleton-line",
            style: "width: {props.width}; height: {props.height};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonCircleProps {
    #[props(default = "32px".to_string())]
    pub size: String,
}

#[component]
pub fn SkeletonCircle(props: SkeletonCircleProps) -> Element {
    rsx! {
        span {
            class: "ds-skeleton ds-skeleton-circle",
            style: "width: {props.size}; height: {props.size};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonCardProps {
    #[props(default = "180px".to_string())]
    pub height: String,
}

#[component]
pub fn SkeletonCard(props: SkeletonCardProps) -> Element {
    rsx! {
        div {
            class: "ds-skeleton ds-skeleton-card",
            style: "height: {props.height};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonTableRowProps {
    #[props(default = 4)]
    pub cells: u8,
}

#[component]
pub fn SkeletonTableRow(props: SkeletonTableRowProps) -> Element {
    let cells: Vec<u8> = (0..props.cells).collect();
    rsx! {
        tr { class: "ds-skeleton-table-row",
            for _ in cells {
                td { SkeletonLine { width: "80%".to_string() } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_line_renders_with_class() {
        fn app() -> Element {
            rsx! { SkeletonLine {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-skeleton"));
        assert!(html.contains("ds-skeleton-line"));
    }

    #[test]
    fn skeleton_circle_renders_with_class() {
        fn app() -> Element {
            rsx! { SkeletonCircle { size: "24px".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-skeleton-circle"));
        assert!(html.contains("width: 24px"));
    }

    #[test]
    fn skeleton_table_row_renders_n_cells() {
        fn app() -> Element {
            rsx! { SkeletonTableRow { cells: 3 } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        let td_count = html.matches("<td>").count();
        assert_eq!(td_count, 3, "expected 3 cells in: {html}");
    }
}
