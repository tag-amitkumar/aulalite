// crates/design-system/src/dropdown_menu.rs
use dioxus::prelude::*;

/// Where the content aligns relative to the trigger.
#[derive(Clone, PartialEq, Default)]
pub enum DropdownAlign {
    #[default]
    Start, // left edge of content aligns with left edge of trigger
    End, // right edge aligns
}

/// Composition-style dropdown menu.
///
/// The caller owns:
/// - **Toggle**: attach a click handler to the trigger that flips `open`.
/// - **Outside-click dismissal**: detect clicks outside the dropdown and set `open` to false.
///
/// This component handles Escape-to-close, auto-focus on first item when
/// opened, and ArrowUp/ArrowDown/Home/End cycling between `role="menuitem"`
/// children — matching WAI-ARIA Authoring Practices for menus.
#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuProps {
    /// Open state. Caller owns it (e.g. `let open = use_signal(|| false);`).
    pub open: Signal<bool>,
    /// Trigger element (button, etc.). Click handler is attached by caller.
    pub trigger: Element,
    /// Menu content (typically <DropdownMenuItem ...>).
    pub children: Element,
    #[props(default)]
    pub align: DropdownAlign,
}

#[component]
pub fn DropdownMenu(props: DropdownMenuProps) -> Element {
    let is_open = *props.open.read();
    let mut open_signal = props.open;
    let align_class = match props.align {
        DropdownAlign::Start => "ds-dropdown-content--start",
        DropdownAlign::End => "ds-dropdown-content--end",
    };
    let state = if is_open { "open" } else { "closed" };
    rsx! {
        div { class: "ds-dropdown",
            {props.trigger}
            if is_open {
                div {
                    role: "menu",
                    class: "ds-dropdown-content {align_class}",
                    "data-state": "{state}",
                    tabindex: "-1",
                    onkeydown: move |evt: KeyboardEvent| {
                        let key = evt.key().to_string();
                        if key == "Escape" {
                            evt.prevent_default();
                            open_signal.set(false);
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            move_menu_focus(&evt, &key);
                        }
                    },
                    onmounted: |evt| {
                        spawn(async move {
                            let _ = evt.set_focus(true).await;
                            // Move focus from the menu container to the
                            // first enabled item once it has rendered.
                            #[cfg(target_arch = "wasm32")]
                            {
                                focus_first_menuitem();
                            }
                        });
                    },
                    {props.children}
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn focus_first_menuitem() {
    use wasm_bindgen::JsCast;
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(Some(first)) =
            doc.query_selector(".ds-dropdown-content [role='menuitem']:not([disabled])")
        {
            if let Some(el) = first.dyn_ref::<web_sys::HtmlElement>() {
                let _ = el.focus();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn move_menu_focus(_evt: &KeyboardEvent, key: &str) {
    use wasm_bindgen::JsCast;
    let doc = match web_sys::window().and_then(|w| w.document()) {
        Some(d) => d,
        None => return,
    };
    let nodes =
        match doc.query_selector_all(".ds-dropdown-content [role='menuitem']:not([disabled])") {
            Ok(n) => n,
            Err(_) => return,
        };
    let len = nodes.length() as i32;
    if len == 0 {
        return;
    }
    let mut items: Vec<web_sys::HtmlElement> = Vec::with_capacity(len as usize);
    for i in 0..len {
        if let Some(node) = nodes.item(i as u32) {
            if let Some(el) = node.dyn_ref::<web_sys::HtmlElement>() {
                items.push(el.clone());
            }
        }
    }
    let active = doc.active_element();
    let current = active
        .as_ref()
        .and_then(|el| {
            items
                .iter()
                .position(|it| it.is_same_node(Some(el.as_ref())))
        })
        .map(|p| p as i32)
        .unwrap_or(-1);
    let next = match key {
        "ArrowDown" => Some(if current < 0 { 0 } else { (current + 1) % len }),
        "ArrowUp" => Some(if current <= 0 { len - 1 } else { current - 1 }),
        "Home" => Some(0),
        "End" => Some(len - 1),
        _ => None,
    };
    if let Some(idx) = next {
        if let Some(el) = items.get(idx as usize) {
            let _ = el.focus();
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub enum DropdownItemTone {
    #[default]
    Default,
    Danger,
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuItemProps {
    pub label: String,
    pub on_select: EventHandler<MouseEvent>,
    #[props(default)]
    pub tone: DropdownItemTone,
    #[props(default)]
    pub disabled: bool,
    /// Optional icon glyph (rendered before the label).
    #[props(default)]
    pub leading_icon: Option<Element>,
}

#[component]
pub fn DropdownMenuItem(props: DropdownMenuItemProps) -> Element {
    let tone_class = match props.tone {
        DropdownItemTone::Default => "",
        DropdownItemTone::Danger => " ds-dropdown-item--danger",
    };
    let class = format!("ds-dropdown-item{tone_class}");
    rsx! {
        button {
            role: "menuitem",
            r#type: "button",
            class: "{class}",
            disabled: props.disabled,
            onclick: move |evt| props.on_select.call(evt),
            if let Some(icon) = &props.leading_icon {
                span { class: "ds-dropdown-item-icon", {icon.clone()} }
            }
            "{props.label}"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuLabelProps {
    pub children: Element,
}
#[component]
pub fn DropdownMenuLabel(props: DropdownMenuLabelProps) -> Element {
    rsx! { div { class: "ds-dropdown-label", role: "presentation", {props.children} } }
}

#[component]
pub fn DropdownMenuSeparator() -> Element {
    rsx! { hr { class: "ds-dropdown-separator", role: "separator" } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropdown_hidden_when_closed() {
        fn app() -> Element {
            let open = use_signal(|| false);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "a".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("ds-dropdown-content"),
            "content should not render when closed: {html}"
        );
    }

    #[test]
    fn dropdown_renders_content_when_open() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "Sign out".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-dropdown-content"),
            "content missing: {html}"
        );
        assert!(html.contains("Sign out"));
        assert!(html.contains("data-state=\"open\""));
    }

    #[test]
    fn dropdown_align_end_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    align: DropdownAlign::End,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "a".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-dropdown-content--end"),
            "end alignment missing: {html}"
        );
    }

    #[test]
    fn dropdown_item_danger_tone_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "Delete".to_string(), tone: DropdownItemTone::Danger, on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-dropdown-item--danger"),
            "danger class missing: {html}"
        );
    }
}
