// crates/design-system/src/tabs.rs
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub struct Tab {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}

#[derive(Clone, PartialEq, Default)]
pub enum TabsVariant {
    #[default]
    Underline,
    Pill,
}

#[derive(Props, Clone, PartialEq)]
pub struct TabsProps {
    pub tabs: Vec<Tab>,
    pub active: String,
    pub on_change: EventHandler<String>,
    #[props(default)]
    pub variant: TabsVariant,
}

#[component]
pub fn Tabs(props: TabsProps) -> Element {
    let variant_class = match props.variant {
        TabsVariant::Underline => "ds-tabs ds-tabs--underline",
        TabsVariant::Pill => "ds-tabs ds-tabs--pill",
    };
    let enabled_keys: Vec<String> = props
        .tabs
        .iter()
        .filter(|t| !t.disabled)
        .map(|t| t.key.clone())
        .collect();
    rsx! {
        div { class: "{variant_class}", role: "tablist",
            for tab in &props.tabs {
                {
                    let key = tab.key.clone();
                    let key_for_class = tab.key.clone();
                    let label = tab.label.clone();
                    let disabled = tab.disabled;
                    let active = props.active.clone();
                    let is_active = active == key_for_class;
                    let handler = props.on_change;
                    let handler_for_keys = props.on_change;
                    let enabled = enabled_keys.clone();
                    let key_for_keys = tab.key.clone();
                    rsx! {
                        button {
                            class: if is_active { "ds-tab ds-tab--active" } else { "ds-tab" },
                            role: "tab",
                            r#type: "button",
                            "aria-selected": if is_active { "true" } else { "false" },
                            tabindex: if is_active { "0" } else { "-1" },
                            disabled,
                            onclick: move |_| handler.call(key.clone()),
                            onkeydown: move |e| {
                                let key_str = e.key().to_string();
                                let mut idx = enabled.iter().position(|k| k == &key_for_keys).unwrap_or(0);
                                if key_str == "ArrowRight" || key_str == "ArrowDown" {
                                    idx = (idx + 1) % enabled.len().max(1);
                                    if let Some(k) = enabled.get(idx) { handler_for_keys.call(k.clone()); }
                                } else if key_str == "ArrowLeft" || key_str == "ArrowUp" {
                                    idx = if idx == 0 { enabled.len().saturating_sub(1) } else { idx - 1 };
                                    if let Some(k) = enabled.get(idx) { handler_for_keys.call(k.clone()); }
                                } else if key_str == "Home" {
                                    if let Some(k) = enabled.first() { handler_for_keys.call(k.clone()); }
                                } else if key_str == "End" {
                                    if let Some(k) = enabled.last() { handler_for_keys.call(k.clone()); }
                                }
                            },
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_tab_renders_disabled_attribute() {
        fn app() -> Element {
            rsx! {
                Tabs {
                    tabs: vec![
                        Tab { key: "a".to_string(), label: "Open".to_string(), ..Default::default() },
                        Tab { key: "b".to_string(), label: "Locked".to_string(), disabled: true },
                    ],
                    active: "a".to_string(),
                    on_change: |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("disabled"),
            "disabled attribute missing: {html}"
        );
    }

    #[test]
    fn tabs_pill_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Tabs {
                    variant: TabsVariant::Pill,
                    tabs: vec![Tab { key: "a".to_string(), label: "A".to_string(), ..Default::default() }],
                    active: "a".to_string(),
                    on_change: |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-tabs--pill"), "pill class missing: {html}");
    }

    #[test]
    fn tabs_emit_tablist_role() {
        fn app() -> Element {
            rsx! {
                Tabs {
                    tabs: vec![Tab { key: "a".to_string(), label: "A".to_string(), ..Default::default() }],
                    active: "a".to_string(),
                    on_change: |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("role=\"tablist\""),
            "tablist role missing: {html}"
        );
        assert!(html.contains("role=\"tab\""), "tab role missing: {html}");
        assert!(
            html.contains("aria-selected=\"true\""),
            "aria-selected missing: {html}"
        );
    }
}
