use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CardListProps {
    pub children: Element,
    /// Fixed number of columns at default viewport. If None, uses auto-fit / 260px min.
    #[props(default)]
    pub columns: Option<u8>,
}

#[component]
pub fn CardList(props: CardListProps) -> Element {
    let style = if let Some(cols) = props.columns {
        format!("--ds-card-list-columns: {};", cols)
    } else {
        String::new()
    };
    rsx! {
        ul {
            class: "ds-card-list",
            style: "{style}",
            {props.children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_list_renders_with_class() {
        fn app() -> Element {
            rsx! {
                CardList {
                    li { "a" }
                    li { "b" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card-list"));
    }

    #[test]
    fn card_list_columns_sets_css_var() {
        fn app() -> Element {
            rsx! {
                CardList {
                    columns: 3,
                    li { "a" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("--ds-card-list-columns: 3"),
            "missing css var: {html}"
        );
    }
}
