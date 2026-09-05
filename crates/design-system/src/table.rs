use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct TableProps {
    pub head: Element,
    pub body: Element,
    #[props(default)]
    pub compact: bool,
    #[props(default)]
    pub sticky_header: bool,
    #[props(default)]
    pub striped: bool,
    /// Optional toolbar slot rendered above the <thead> inside the table card.
    #[props(default)]
    pub toolbar: Option<Element>,
}

#[component]
pub fn Table(props: TableProps) -> Element {
    let mut class = String::from("ds-table");
    if props.compact {
        class.push_str(" ds-table--compact");
    }
    if props.sticky_header {
        class.push_str(" ds-table--sticky-head");
    }
    if props.striped {
        class.push_str(" ds-table--striped");
    }
    rsx! {
        div { class: "ds-table-shell",
            if let Some(toolbar) = props.toolbar {
                div { class: "ds-table-toolbar", {toolbar} }
            }
            table { class: "{class}",
                thead { {props.head} }
                tbody { {props.body} }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Props, Clone, PartialEq)]
pub struct TableHeaderCellProps {
    pub children: Element,
    #[props(default)]
    pub sortable: bool,
    #[props(default)]
    pub sort_dir: Option<SortDir>,
    /// Fired when the header is clicked (sortable cells only).
    #[props(default)]
    pub on_sort: Option<EventHandler<MouseEvent>>,
}

#[component]
pub fn TableHeaderCell(props: TableHeaderCellProps) -> Element {
    let aria_sort = match props.sort_dir {
        Some(SortDir::Asc) => "ascending",
        Some(SortDir::Desc) => "descending",
        None => "none",
    };
    let chevron_class = match props.sort_dir {
        Some(SortDir::Desc) => "ds-table-sort-chevron ds-table-sort-chevron--desc",
        _ => "ds-table-sort-chevron",
    };
    if props.sortable {
        rsx! {
            th { class: "ds-table-th ds-table-th--sortable", "aria-sort": "{aria_sort}",
                button {
                    r#type: "button",
                    class: "ds-table-sort-btn",
                    onclick: move |evt| {
                        if let Some(h) = &props.on_sort { h.call(evt); }
                    },
                    {props.children}
                    span { class: "{chevron_class}", "aria-hidden": "true", "▾" }
                }
            }
        }
    } else {
        rsx! { th { class: "ds-table-th", {props.children} } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_renders_default_class() {
        fn app() -> Element {
            rsx! {
                Table {
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-table"));
        assert!(html.contains("<thead>"));
        assert!(html.contains("<tbody>"));
    }

    #[test]
    fn table_compact_renders_class() {
        fn app() -> Element {
            rsx! {
                Table {
                    compact: true,
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-table--compact"));
    }

    #[test]
    fn table_sticky_header_renders_class() {
        fn app() -> Element {
            rsx! {
                Table {
                    sticky_header: true,
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-table--sticky-head"));
    }

    #[test]
    fn table_striped_renders_class() {
        fn app() -> Element {
            rsx! {
                Table {
                    striped: true,
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-table--striped"),
            "striped class missing: {html}"
        );
    }

    #[test]
    fn table_toolbar_renders_above_head() {
        fn app() -> Element {
            rsx! {
                Table {
                    toolbar: Some(rsx! { div { "filters here" } }),
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-table-toolbar"),
            "toolbar wrap missing: {html}"
        );
        assert!(html.contains("filters here"));
    }

    #[test]
    fn table_header_cell_sortable_emits_aria() {
        fn app() -> Element {
            rsx! {
                TableHeaderCell {
                    sortable: true,
                    sort_dir: Some(SortDir::Asc),
                    on_sort: |_| {},
                    "Name"
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("aria-sort=\"ascending\""),
            "aria-sort missing: {html}"
        );
        assert!(
            html.contains("ds-table-sort-chevron"),
            "chevron marker missing: {html}"
        );
    }
}
