// crates/features-courses/src/a11y.rs
//! Accessibility primitives shared across the app:
//!
//! * [`SkipToContent`] — a "skip to main content" link that is the first
//!   focusable element on the page. It stays visually hidden until focused
//!   (keyboard / screen-reader users), then animates into view. Activating it
//!   jumps focus past the navigation chrome straight to the main content
//!   region. Mount it once, as the very first child of the app root, and give
//!   the main content container `id="main-content"` (or pass a custom
//!   `target_id`).
//!
//! * [`use_focus_trap`] — a renderer-portable focus-trap hook for modals,
//!   sheets, and drawers. It keeps Tab / Shift+Tab cycling **inside** the
//!   dialog by rendering two invisible focus-guard sentinels around the trapped
//!   region: focusing a sentinel bounces focus to the opposite edge of the
//!   dialog. It relies only on Dioxus' portable `set_focus` (a no-op off the
//!   web renderer), so it stays wasm-safe and host-test-safe — no extra
//!   `web-sys` features required.
//!
//! Both are additive and framework-level: the design-system `Modal`/`Sheet`
//! already wire `role="dialog"`, `aria-modal`, Escape-to-close, and initial
//! auto-focus; this hook layers Tab containment on top for dialogs with many
//! interactive controls.

use dioxus::prelude::*;

/// Default DOM id the [`SkipToContent`] link targets. The app's main content
/// region should carry this id (or you can override via `target_id`).
pub const MAIN_CONTENT_ID: &str = "main-content";

#[derive(Props, Clone, PartialEq)]
pub struct SkipToContentProps {
    /// The id of the main-content landmark to jump to. Defaults to
    /// [`MAIN_CONTENT_ID`].
    #[props(default = MAIN_CONTENT_ID.to_string())]
    pub target_id: String,
    /// Visible link text. Defaults to "Skip to main content".
    #[props(default = "Skip to main content".to_string())]
    pub label: String,
}

/// A WCAG 2.4.1 "bypass blocks" skip link.
///
/// Renders an `<a href="#{target_id}">` that is the first focusable element in
/// the tab order. It is visually hidden (off-screen) until it receives focus,
/// at which point `.skip-to-content` (see components.css) reveals it. Clicking
/// / pressing Enter navigates to the in-page anchor.
///
/// For focus (not just scroll) to follow the jump, give the target landmark
/// `tabindex="-1"` (e.g. `<section id="main-content" tabindex="-1">`). Evergreen
/// browsers then move keyboard focus onto the target on hash navigation, so the
/// next Tab continues from the main content rather than the top of the page.
/// This keeps the component dependency-free (no raw DOM access required).
#[component]
pub fn SkipToContent(props: SkipToContentProps) -> Element {
    let href = format!("#{}", props.target_id);
    rsx! {
        a {
            class: "skip-to-content",
            href: "{href}",
            "{props.label}"
        }
    }
}

/// Handles returned by [`use_focus_trap`]. Spread these onto the trapped region
/// to contain Tab focus.
///
/// Usage:
/// ```ignore
/// let trap = use_focus_trap();
/// rsx! {
///     div { role: "dialog", "aria-modal": "true",
///         // Leading sentinel: focusing it (Shift+Tab from the first control)
///         // bounces focus to the last control.
///         {trap.guard_before()}
///         // ...dialog controls...
///         // Trailing sentinel: focusing it (Tab from the last control)
///         // bounces focus to the first control.
///         {trap.guard_after()}
///     }
/// }
/// ```
#[derive(Clone)]
pub struct FocusTrap {
    /// Signal holding the trapped region's first focusable boundary sentinel.
    first: Signal<Option<std::rc::Rc<MountedData>>>,
    /// Signal holding the trapped region's last focusable boundary sentinel.
    last: Signal<Option<std::rc::Rc<MountedData>>>,
}

impl FocusTrap {
    /// The leading focus-guard sentinel. Render it as the FIRST child inside the
    /// trapped region. When focus lands on it (Shift+Tab wrapping backwards), it
    /// redirects focus to the trailing edge of the dialog.
    pub fn guard_before(&self) -> Element {
        let last = self.last;
        let mut first = self.first;
        rsx! {
            div {
                class: "a11y-focus-guard",
                tabindex: "0",
                "aria-hidden": "true",
                onmounted: move |evt: Event<MountedData>| {
                    first.set(Some(evt.data()));
                },
                onfocus: move |_| {
                    if let Some(node) = last.read().clone() {
                        spawn(async move {
                            let _ = node.set_focus(true).await;
                        });
                    }
                },
            }
        }
    }

    /// The trailing focus-guard sentinel. Render it as the LAST child inside the
    /// trapped region. When focus lands on it (Tab wrapping forwards), it
    /// redirects focus to the leading edge of the dialog.
    pub fn guard_after(&self) -> Element {
        let first = self.first;
        let mut last = self.last;
        rsx! {
            div {
                class: "a11y-focus-guard",
                tabindex: "0",
                "aria-hidden": "true",
                onmounted: move |evt: Event<MountedData>| {
                    last.set(Some(evt.data()));
                },
                onfocus: move |_| {
                    if let Some(node) = first.read().clone() {
                        spawn(async move {
                            let _ = node.set_focus(true).await;
                        });
                    }
                },
            }
        }
    }
}

/// A renderer-portable focus-trap hook for dialogs / sheets / drawers.
///
/// Returns a [`FocusTrap`] whose `guard_before` / `guard_after` sentinels you
/// render as the first and last children of the trapped region. Focusing a
/// sentinel (which only happens when Tab / Shift+Tab tries to leave the dialog)
/// bounces focus to the opposite edge, so keyboard focus never escapes the
/// dialog while it is open.
///
/// This composes with the design-system `Modal`/`Sheet`, which already set
/// `role="dialog"`, `aria-modal="true"`, Escape-to-close, and auto-focus on
/// mount. It adds nothing on the host (SSR) renderer beyond two hidden,
/// `aria-hidden` divs, so it is safe in tests.
pub fn use_focus_trap() -> FocusTrap {
    let first = use_signal(|| None);
    let last = use_signal(|| None);
    FocusTrap { first, last }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_link_renders_anchor_to_default_target() {
        fn app() -> Element {
            rsx! { SkipToContent {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("class=\"skip-to-content\""),
            "skip link class missing: {html}"
        );
        assert!(
            html.contains("href=\"#main-content\""),
            "skip link should target #main-content: {html}"
        );
        assert!(
            html.contains("Skip to main content"),
            "skip link label missing: {html}"
        );
    }

    #[test]
    fn skip_link_honors_custom_target_and_label() {
        fn app() -> Element {
            rsx! {
                SkipToContent {
                    target_id: "app-content".to_string(),
                    label: "Jump to content".to_string(),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("href=\"#app-content\""),
            "custom target missing: {html}"
        );
        assert!(
            html.contains("Jump to content"),
            "custom label missing: {html}"
        );
    }

    #[test]
    fn focus_trap_guards_render_as_hidden_sentinels() {
        fn app() -> Element {
            let trap = use_focus_trap();
            rsx! {
                div { role: "dialog", "aria-modal": "true",
                    {trap.guard_before()}
                    button { "ok" }
                    {trap.guard_after()}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Both sentinels render, are focusable (tabindex=0) and hidden from AT.
        let guard_count = html.matches("a11y-focus-guard").count();
        assert_eq!(guard_count, 2, "expected two focus-guard sentinels: {html}");
        assert!(
            html.contains("tabindex=\"0\""),
            "guards must be focusable: {html}"
        );
        assert!(
            html.contains("aria-hidden=\"true\""),
            "guards must be hidden from assistive tech: {html}"
        );
        // The real control sits between the guards.
        assert!(html.contains(">ok<"), "trapped control missing: {html}");
    }
}
