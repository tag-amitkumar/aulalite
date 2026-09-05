use dioxus::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SHEET_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
struct SheetFocusReturn {
    #[cfg(target_arch = "wasm32")]
    previous: std::rc::Rc<std::cell::RefCell<Option<web_sys::HtmlElement>>>,
}

impl SheetFocusReturn {
    fn capture(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            let previous = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element())
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
            *self.previous.borrow_mut() = previous;
        }
    }

    fn restore(&self) {
        #[cfg(target_arch = "wasm32")]
        if let Some(previous) = self.previous.borrow_mut().take() {
            let _ = previous.focus();
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub enum SheetSide {
    #[default]
    Right,
    Left,
    Top,
    Bottom,
}

/// Side drawer (Sheet) primitive.
///
/// The caller owns:
/// - **Toggle**: attach a click handler to the trigger that flips `open`.
///
/// The Sheet itself handles backdrop-click, Escape close, and auto-focuses
/// the panel on open so screen-reader users and keyboard-only users are
/// inside the dialog from the first tick.
///
#[derive(Props, Clone, PartialEq)]
pub struct SheetProps {
    pub open: Signal<bool>,
    pub children: Element,
    #[props(default)]
    pub side: SheetSide,
    /// Width in pixels for left/right sheets (default 440). Ignored for top/bottom.
    #[props(default = 440)]
    pub width: u32,
    /// Height in pixels for top/bottom sheets (default 320). Ignored for left/right.
    #[props(default = 320)]
    pub height: u32,
    /// Accessible name announced for the dialog. Production call sites should
    /// describe the panel's purpose rather than relying on visible layout.
    #[props(default = "Panel".to_string())]
    pub aria_label: String,
}

#[component]
pub fn Sheet(props: SheetProps) -> Element {
    let sheet_id =
        use_hook(|| format!("ds-sheet-{}", NEXT_SHEET_ID.fetch_add(1, Ordering::Relaxed)));
    let focus_return = use_hook(SheetFocusReturn::default);
    use_context_provider(|| focus_return.clone());
    let restore_on_drop = focus_return.clone();
    use_drop(move || restore_on_drop.restore());

    let is_open = *props.open.read();
    if !is_open {
        return rsx! {};
    }
    let side_class = match props.side {
        SheetSide::Right => "ds-sheet--right",
        SheetSide::Left => "ds-sheet--left",
        SheetSide::Top => "ds-sheet--top",
        SheetSide::Bottom => "ds-sheet--bottom",
    };
    let inline_style = match props.side {
        SheetSide::Right | SheetSide::Left => format!("width: {}px;", props.width),
        SheetSide::Top | SheetSide::Bottom => format!("height: {}px;", props.height),
    };
    let mut open_signal = props.open;
    let restore_backdrop = focus_return.clone();
    let restore_escape = focus_return.clone();
    let capture_focus = focus_return.clone();
    #[cfg(target_arch = "wasm32")]
    let trap_id = sheet_id.clone();
    rsx! {
        div { class: "ds-sheet-backdrop",
            "data-state": "open",
            onclick: move |_| {
                restore_backdrop.restore();
                open_signal.set(false);
            },
            // Panel; clicks inside should NOT close.
            div {
                id: "{sheet_id}",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "{props.aria_label}",
                class: "ds-sheet {side_class}",
                style: "{inline_style}",
                "data-state": "open",
                onclick: move |evt| { evt.stop_propagation(); },
                onkeydown: move |evt: KeyboardEvent| {
                    let key = evt.key().to_string();
                    if key == "Escape" {
                        evt.prevent_default();
                        restore_escape.restore();
                        open_signal.set(false);
                    } else if key == "Tab" {
                        #[cfg(target_arch = "wasm32")]
                        if trap_sheet_focus(
                            &trap_id,
                            evt.modifiers().contains(Modifiers::SHIFT),
                        ) {
                            evt.prevent_default();
                        }
                    }
                },
                // Auto-focus the panel on mount so the dialog receives
                // keyboard events immediately. `tabindex=-1` lets us focus
                // it programmatically without inserting a tab stop in the
                // natural keyboard order.
                onmounted: move |evt| {
                    capture_focus.capture();
                    spawn(async move {
                        let _ = evt.set_focus(true).await;
                    });
                },
                tabindex: "-1",
                {props.children}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetSlotProps {
    pub children: Element,
}

#[component]
pub fn SheetHeader(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-header", {props.children} } }
}
#[component]
pub fn SheetTitle(props: SheetSlotProps) -> Element {
    rsx! { h2 { class: "ds-sheet-title", {props.children} } }
}
#[component]
pub fn SheetDescription(props: SheetSlotProps) -> Element {
    rsx! { p { class: "ds-sheet-description", {props.children} } }
}
#[component]
pub fn SheetBody(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-body", {props.children} } }
}
#[component]
pub fn SheetFooter(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-footer", {props.children} } }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetCloseProps {
    pub open: Signal<bool>,
    #[props(default = "Close".to_string())]
    pub label: String,
}

#[component]
pub fn SheetClose(props: SheetCloseProps) -> Element {
    let mut open = props.open;
    let focus_return = try_consume_context::<SheetFocusReturn>();
    rsx! {
        button {
            r#type: "button",
            class: "ds-sheet-close",
            "aria-label": "{props.label}",
            onclick: move |_| {
                if let Some(focus_return) = &focus_return {
                    focus_return.restore();
                }
                open.set(false);
            },
            "\u{00d7}"
        }
    }
}

/// Cycle focus inside the open sheet and report whether normal Tab behavior
/// must be cancelled because focus wrapped to the opposite edge.
#[cfg(target_arch = "wasm32")]
fn trap_sheet_focus(sheet_id: &str, backwards: bool) -> bool {
    use wasm_bindgen::JsCast;

    let document = match web_sys::window().and_then(|window| window.document()) {
        Some(document) => document,
        None => return false,
    };
    let panel = match document.get_element_by_id(sheet_id) {
        Some(panel) => panel,
        None => return false,
    };
    let nodes = match panel.query_selector_all(
        "a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex='-1'])",
    ) {
        Ok(nodes) => nodes,
        Err(_) => return false,
    };
    let mut focusable = Vec::<web_sys::HtmlElement>::new();
    for index in 0..nodes.length() {
        if let Some(element) = nodes
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        {
            focusable.push(element);
        }
    }
    if focusable.is_empty() {
        if let Some(panel) = panel.dyn_ref::<web_sys::HtmlElement>() {
            let _ = panel.focus();
        }
        return true;
    }

    let active = document.active_element();
    let current = active.as_ref().and_then(|active| {
        focusable
            .iter()
            .position(|element| element.is_same_node(Some(active.as_ref())))
    });
    let target = if backwards {
        if current.is_none() || current == Some(0) {
            focusable.last()
        } else {
            None
        }
    } else if current.is_none() || current == Some(focusable.len() - 1) {
        focusable.first()
    } else {
        None
    };
    if let Some(target) = target {
        let _ = target.focus();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_renders_nothing_when_closed() {
        fn app() -> Element {
            let open = use_signal(|| false);
            rsx! { Sheet { open, "body" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("ds-sheet-backdrop"),
            "should not render when closed: {html}"
        );
    }

    #[test]
    fn sheet_renders_panel_when_open() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! { Sheet { open, SheetHeader { SheetTitle { "Edit" } } } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-sheet-backdrop"));
        assert!(
            html.contains("ds-sheet--right"),
            "default side missing: {html}"
        );
        assert!(html.contains("ds-sheet-title"));
        assert!(html.contains("aria-modal=\"true\""));
        assert!(html.contains("aria-label=\"Panel\""));
        assert!(html.contains("tabindex=\"-1\""));
    }

    #[test]
    fn sheet_uses_caller_supplied_accessible_name() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! { Sheet { open, aria_label: "Delivery details", "body" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aria-label=\"Delivery details\""), "{html}");
    }

    #[test]
    fn sheet_left_side_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! { Sheet { open, side: SheetSide::Left, "body" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-sheet--left"),
            "left class missing: {html}"
        );
    }
}
