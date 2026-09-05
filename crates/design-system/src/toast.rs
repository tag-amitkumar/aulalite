use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Danger,
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum ToastPosition {
    #[default]
    BottomRight,
    TopRight,
    BottomCenter,
    TopCenter,
}

#[derive(Clone, PartialEq)]
pub struct ToastEntry {
    pub id: u64,
    pub level: ToastLevel,
    pub title: String,
    pub message: String,
    pub duration_ms: Option<u32>,
}

pub type ToastQueue = Vec<ToastEntry>;

fn next_toast_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub struct ToastSender(pub Signal<ToastQueue>);

impl ToastSender {
    pub fn push(
        &mut self,
        level: ToastLevel,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let id = next_toast_id();
        let duration_ms = 4000u32;
        self.0.write().push(ToastEntry {
            id,
            level,
            title: title.into(),
            message: message.into(),
            duration_ms: Some(duration_ms),
        });

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::closure::Closure;
            use wasm_bindgen::JsCast;
            let mut sender = *self;
            let closure = Closure::once_into_js(move || {
                sender.dismiss(id);
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    duration_ms as i32,
                );
            }
        }
    }
    pub fn dismiss(&mut self, id: u64) {
        self.0.write().retain(|e| e.id != id);
    }
}

/// Acquire the toast sender from context. Must be called inside a tree
/// that mounted [`ToastProvider`].
pub fn use_toast_sender() -> ToastSender {
    let signal = use_context::<Signal<ToastQueue>>();
    ToastSender(signal)
}

#[component]
pub fn ToastViewport() -> Element {
    let queue = use_context::<Signal<ToastQueue>>();
    let entries = queue.read().clone();
    // Try to read a position from context; default to BottomRight if absent.
    let position = try_consume_context::<ToastPosition>().unwrap_or_default();
    let position_class = match position {
        ToastPosition::BottomRight => "ds-toast-viewport--bottom-right",
        ToastPosition::TopRight => "ds-toast-viewport--top-right",
        ToastPosition::BottomCenter => "ds-toast-viewport--bottom-center",
        ToastPosition::TopCenter => "ds-toast-viewport--top-center",
    };
    rsx! {
        div { class: "ds-toast-viewport {position_class}",
            for entry in entries.iter() {
                {
                    let id = entry.id;
                    let level_class = match entry.level {
                        ToastLevel::Info     => "ds-toast--info",
                        ToastLevel::Success  => "ds-toast--success",
                        ToastLevel::Warning  => "ds-toast--warning",
                        ToastLevel::Danger   => "ds-toast--danger",
                        ToastLevel::Premium  => "ds-toast--premium",
                    };
                    // Warning/Danger toasts use `role="alert"` + assertive
                    // live region so screen readers announce them
                    // immediately. Lower-priority toasts stay polite to
                    // avoid interrupting the user's current task.
                    let (role, aria_live) = match entry.level {
                        ToastLevel::Danger | ToastLevel::Warning => ("alert", "assertive"),
                        _ => ("status", "polite"),
                    };
                    let title = entry.title.clone();
                    let message = entry.message.clone();
                    rsx! {
                        div {
                            key: "{id}",
                            class: "ds-toast {level_class}",
                            role: "{role}",
                            "aria-live": "{aria_live}",
                            "aria-atomic": "true",
                            div { class: "ds-toast-title", "{title}" }
                            div { class: "ds-toast-message", "{message}" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToastProviderProps {
    pub children: Element,
    #[props(default)]
    pub position: ToastPosition,
}

/// App-root provider. Call once near the top of the component tree.
#[component]
pub fn ToastProvider(props: ToastProviderProps) -> Element {
    use_context_provider(|| Signal::new(ToastQueue::new()));
    use_context_provider(|| props.position.clone());
    rsx! {
        {props.children}
        ToastViewport {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_renders_empty_when_no_entries() {
        fn app() -> Element {
            use_context_provider(|| Signal::new(ToastQueue::new()));
            rsx! { ToastViewport {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-toast-viewport"),
            "viewport class missing: {html}"
        );
        // No toast entries
        assert!(!html.contains("ds-toast--info"));
    }

    #[test]
    fn viewport_renders_entry_with_level_class() {
        fn app() -> Element {
            let queue = use_context_provider(|| {
                Signal::new(vec![ToastEntry {
                    id: 1,
                    level: ToastLevel::Success,
                    title: "Saved".to_string(),
                    message: "Course updated".to_string(),
                    duration_ms: None,
                }])
            });
            let _ = queue;
            rsx! { ToastViewport {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-toast--success"),
            "success class missing: {html}"
        );
        assert!(html.contains("Saved"));
        assert!(html.contains("Course updated"));
    }

    #[test]
    fn push_stamps_4000ms_duration() {
        // Mount a provider-equivalent context so use_toast_sender can find the signal.
        fn app() -> Element {
            use_context_provider(|| Signal::new(ToastQueue::new()));
            // Use a side effect within the component to push and assert.
            let signal = use_context::<Signal<ToastQueue>>();
            let mut sender = ToastSender(signal);
            sender.push(ToastLevel::Info, "title", "message");
            let queue = use_context::<Signal<ToastQueue>>();
            let entries = queue.read();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].duration_ms, Some(4000));
            rsx! { div { "ok" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ok"));
    }

    #[test]
    fn viewport_renders_premium_level_class() {
        fn app() -> Element {
            let _ = use_context_provider(|| {
                Signal::new(vec![ToastEntry {
                    id: 1,
                    level: ToastLevel::Premium,
                    title: "Pro unlocked".to_string(),
                    message: "Enjoy 30 days free".to_string(),
                    duration_ms: None,
                }])
            });
            rsx! { ToastViewport {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-toast--premium"),
            "premium class missing: {html}"
        );
    }

    #[test]
    fn viewport_has_position_class_top_right_when_provider_set() {
        fn app() -> Element {
            rsx! {
                ToastProvider { position: ToastPosition::TopRight,
                    div { "child" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-toast-viewport--top-right"),
            "top-right class missing: {html}"
        );
    }
}
