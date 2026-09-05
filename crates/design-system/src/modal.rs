// crates/design-system/src/modal.rs
use dioxus::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MODAL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
struct FocusReturn {
    #[cfg(target_arch = "wasm32")]
    previous: std::rc::Rc<std::cell::RefCell<Option<web_sys::HtmlElement>>>,
}

impl FocusReturn {
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
pub enum ModalSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Props, Clone, PartialEq)]
pub struct ModalProps {
    pub open: bool,
    pub title: String,
    pub on_close: EventHandler<()>,
    #[props(default)]
    pub size: ModalSize,
    pub children: Element,
}

#[component]
pub fn Modal(props: ModalProps) -> Element {
    let modal_id =
        use_hook(|| format!("ds-modal-{}", NEXT_MODAL_ID.fetch_add(1, Ordering::Relaxed)));
    let title_id = format!("{modal_id}-title");
    let focus_return = use_hook(FocusReturn::default);
    let restore_on_drop = focus_return.clone();
    use_drop(move || restore_on_drop.restore());

    if !props.open {
        return rsx!({});
    }
    let size_class = match props.size {
        ModalSize::Small => "modal modal--small",
        ModalSize::Medium => "modal modal--medium",
        ModalSize::Large => "modal modal--large",
    };
    let close_backdrop = props.on_close;
    let close_escape = props.on_close;
    let close_button = props.on_close;
    let restore_backdrop = focus_return.clone();
    let restore_escape = focus_return.clone();
    let restore_button = focus_return.clone();
    let capture_focus = focus_return.clone();
    let _trap_id = modal_id.clone();
    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| {
                restore_backdrop.restore();
                close_backdrop.call(());
            },
            div {
                id: "{modal_id}",
                class: "{size_class}",
                onclick: |evt| evt.stop_propagation(),
                role: "dialog",
                "aria-modal": "true",
                "aria-labelledby": "{title_id}",
                // `tabindex=-1` lets us focus the dialog programmatically
                // without inserting a tab stop in the natural keyboard order.
                tabindex: "-1",
                // Escape-to-close for keyboard users (parity with Sheet).
                onkeydown: move |evt: KeyboardEvent| {
                    let key = evt.key().to_string();
                    if key == "Escape" {
                        evt.prevent_default();
                        restore_escape.restore();
                        close_escape.call(());
                    } else if key == "Tab" {
                        #[cfg(target_arch = "wasm32")]
                        if trap_modal_focus(
                            &_trap_id,
                            evt.modifiers().contains(Modifiers::SHIFT),
                        ) {
                            evt.prevent_default();
                        }
                    }
                },
                // Auto-focus the dialog on mount so keyboard / screen-reader
                // users land inside it and Escape is delivered here. Uses the
                // portable `set_focus` (no-op off the web renderer), so it
                // stays wasm-safe and host-test-safe.
                onmounted: move |evt| {
                    capture_focus.capture();
                    spawn(async move {
                        let _ = evt.set_focus(true).await;
                    });
                },
                header { class: "modal-header",
                    h2 { id: "{title_id}", class: "modal-title", "{props.title}" }
                    button {
                        r#type: "button",
                        class: "modal-close",
                        "aria-label": "Close",
                        onclick: move |_| {
                            restore_button.restore();
                            close_button.call(());
                        },
                        "×"
                    }
                }
                div { class: "modal-body", {props.children} }
            }
        }
    }
}

/// Cycle focus inside the current modal. Returns `true` when the browser's
/// normal Tab behavior must be cancelled because focus was wrapped.
#[cfg(target_arch = "wasm32")]
fn trap_modal_focus(modal_id: &str, backwards: bool) -> bool {
    use wasm_bindgen::JsCast;

    let document = match web_sys::window().and_then(|window| window.document()) {
        Some(document) => document,
        None => return false,
    };
    let dialog = match document.get_element_by_id(modal_id) {
        Some(dialog) => dialog,
        None => return false,
    };
    let nodes = match dialog.query_selector_all(
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
        if let Some(dialog) = dialog.dyn_ref::<web_sys::HtmlElement>() {
            let _ = dialog.focus();
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
    use dioxus::prelude::VirtualDom;

    #[test]
    fn modal_renders_default_size() {
        fn app() -> Element {
            rsx! {
                Modal {
                    open: true,
                    title: "Hi".to_string(),
                    on_close: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("modal--medium"),
            "default size class missing: {html}"
        );
    }

    #[test]
    fn modal_small_size_renders_class() {
        fn app() -> Element {
            rsx! {
                Modal {
                    open: true,
                    title: "Hi".to_string(),
                    size: ModalSize::Small,
                    on_close: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("modal--small"),
            "small size class missing: {html}"
        );
    }

    #[test]
    fn modal_large_size_renders_class() {
        fn app() -> Element {
            rsx! {
                Modal {
                    open: true,
                    title: "Hi".to_string(),
                    size: ModalSize::Large,
                    on_close: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("modal--large"),
            "large size class missing: {html}"
        );
    }

    #[test]
    fn modal_open_wires_escape_and_focus_affordances() {
        fn app() -> Element {
            rsx! {
                Modal {
                    open: true,
                    title: "Hi".to_string(),
                    on_close: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // tabindex makes the dialog programmatically focusable (auto-focus on
        // mount) and is the marker that the Escape/focus affordances are wired.
        assert!(
            html.contains("tabindex=\"-1\""),
            "dialog should be focusable (tabindex=-1) for Escape/focus support: {html}"
        );
        // The dialog role / aria-modal must remain intact.
        assert!(
            html.contains("role=\"dialog\""),
            "role=dialog missing: {html}"
        );
        assert!(
            html.contains("aria-modal=\"true\""),
            "aria-modal missing: {html}"
        );
        assert!(
            html.contains("aria-labelledby=\"ds-modal-") && html.contains("class=\"modal-title\""),
            "dialog title association missing: {html}"
        );
        assert!(
            html.contains("type=\"button\""),
            "close control must not submit a containing form: {html}"
        );
    }

    #[test]
    fn modal_closed_renders_nothing() {
        fn app() -> Element {
            rsx! {
                Modal {
                    open: false,
                    title: "Hi".to_string(),
                    on_close: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("modal-backdrop"),
            "closed modal should not render backdrop: {html}"
        );
    }
}
