//! CSP-safe command palette and product tour.
//!
//! These mirror the subset of the kinetics overlay API AulaLite uses, but all
//! focus, scrolling, and spotlight work goes through typed DOM APIs. This
//! keeps keyboard behavior intact without JavaScript `unsafe-eval`.

use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
const FOCUSABLE_SELECTOR: &str = "a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex='-1'])";
const SKELETON_ROW_COUNT: usize = 4;

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

#[cfg(target_arch = "wasm32")]
fn trap_focus(panel_id: &str, backwards: bool) -> bool {
    use wasm_bindgen::JsCast;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return false;
    };
    let Some(panel) = document.get_element_by_id(panel_id) else {
        return false;
    };
    let Ok(nodes) = panel.query_selector_all(FOCUSABLE_SELECTOR) else {
        return false;
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

    let current = document.active_element().as_ref().and_then(|active| {
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

#[cfg(target_arch = "wasm32")]
fn scroll_selected_into_view(selected_id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(&format!("ui-command-item-{selected_id}")))
    else {
        return;
    };
    let options = web_sys::ScrollIntoViewOptions::new();
    options.set_block(web_sys::ScrollLogicalPosition::Nearest);
    options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
    element.scroll_into_view_with_scroll_into_view_options(&options);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon_path: String,
    pub shortcut: String,
}

impl CommandItem {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            icon_path: String::new(),
            shortcut: String::new(),
        }
    }

    pub fn with_icon(mut self, path: impl Into<String>) -> Self {
        self.icon_path = path.into();
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandGroup {
    pub label: String,
    pub items: Vec<CommandItem>,
}

impl CommandGroup {
    pub fn new(label: impl Into<String>, items: Vec<CommandItem>) -> Self {
        Self {
            label: label.into(),
            items,
        }
    }
}

#[component]
pub fn CommandMenu(
    #[props(default = "ui-command-menu".to_string())] id: String,
    #[props(default)] open: bool,
    #[props(default)] query: String,
    #[props(default)] selected_id: String,
    #[props(default = "No commands found".to_string())] empty_text: String,
    #[props(default)] loading: bool,
    #[props(default)] groups: Vec<CommandGroup>,
    on_query: Option<EventHandler<String>>,
    on_select: Option<EventHandler<String>>,
    on_selection_change: Option<EventHandler<String>>,
    on_dismiss: Option<EventHandler<()>>,
) -> Element {
    let focus_return = use_hook(FocusReturn::default);
    let restore_on_drop = focus_return.clone();
    use_drop(move || restore_on_drop.restore());

    let _selected_for_scroll = selected_id.clone();
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        if open && !_selected_for_scroll.is_empty() {
            scroll_selected_into_view(&_selected_for_scroll);
        }
    });

    if !open {
        return rsx! {};
    }

    let panel_id = format!("{id}-panel");
    let list_id = format!("{id}-list");
    let item_count: usize = groups.iter().map(|group| group.items.len()).sum();
    let active_descendant = if selected_id.is_empty() {
        String::new()
    } else {
        format!("ui-command-item-{selected_id}")
    };
    let announcement = match (loading, item_count) {
        (true, _) => "Loading commands".to_string(),
        (false, 0) => "No results".to_string(),
        (false, 1) => "1 result".to_string(),
        (false, count) => format!("{count} results"),
    };
    let flat_ids: Vec<String> = groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.id.clone()))
        .collect();
    let flat_ids_for_key = flat_ids.clone();
    let selected_for_key = selected_id.clone();
    let _panel_for_key = panel_id.clone();
    let restore_key = focus_return.clone();
    let restore_backdrop = focus_return.clone();
    let capture_mount = focus_return.clone();

    rsx! {
        div {
            class: "ui-command-menu",
            role: "dialog",
            "aria-modal": "true",
            onkeydown: move |event| {
                match event.key() {
                    Key::Escape => {
                        event.stop_propagation();
                        restore_key.restore();
                        if let Some(handler) = &on_dismiss { handler.call(()); }
                    }
                    Key::Enter if !selected_for_key.is_empty() => {
                        event.prevent_default();
                        if let Some(handler) = &on_select {
                            handler.call(selected_for_key.clone());
                        }
                    }
                    Key::ArrowDown | Key::ArrowUp => {
                        if let Some(handler) = &on_selection_change {
                            if let Some(next) = step_selection(
                                &flat_ids_for_key,
                                &selected_for_key,
                                if event.key() == Key::ArrowDown { 1 } else { -1 },
                            ) {
                                event.prevent_default();
                                handler.call(next);
                            }
                        }
                    }
                    Key::Tab => {
                        #[cfg(target_arch = "wasm32")]
                        if trap_focus(
                            &_panel_for_key,
                            event.modifiers().contains(Modifiers::SHIFT),
                        ) {
                            event.prevent_default();
                        }
                    }
                    _ => {}
                }
            },
            div {
                class: "ui-command-menu-backdrop",
                onclick: move |_| {
                    restore_backdrop.restore();
                    if let Some(handler) = &on_dismiss { handler.call(()); }
                },
            }
            div {
                id: "{panel_id}",
                class: "ui-command-menu-panel",
                "data-state": "open",
                tabindex: "-1",
                onmounted: move |_| capture_mount.capture(),
                input {
                    class: "ui-command-menu-input",
                    value: "{query}",
                    placeholder: "Search commands",
                    "aria-label": "Search commands",
                    "aria-autocomplete": "list",
                    "aria-controls": "{list_id}",
                    "aria-activedescendant": "{active_descendant}",
                    autocomplete: "off",
                    onmounted: move |event| {
                        spawn(async move {
                            let _ = event.set_focus(true).await;
                        });
                    },
                    oninput: move |event| {
                        if let Some(handler) = &on_query { handler.call(event.value()); }
                    },
                }
                div {
                    class: "visually-hidden",
                    role: "status",
                    "aria-live": "polite",
                    "aria-atomic": "true",
                    "{announcement}"
                }
                if loading {
                    div {
                        id: "{list_id}",
                        class: "ui-command-menu-list",
                        role: "listbox",
                        "aria-busy": "true",
                        for row in 0..SKELETON_ROW_COUNT {
                            div {
                                key: "skeleton-{row}",
                                class: "ui-command-menu-item ui-command-menu-item--skeleton",
                                "aria-hidden": "true",
                                span { class: "ui-skeleton ui-command-menu-skeleton-line" }
                            }
                        }
                    }
                } else if item_count > 0 {
                    div {
                        id: "{list_id}",
                        class: "ui-command-menu-list",
                        role: "listbox",
                        "aria-busy": "false",
                        for group in groups {
                            div { class: "ui-command-menu-group",
                                p { class: "ui-command-menu-group-label", "{group.label}" }
                                for item in group.items {
                                    {
                                        let select_id = item.id.clone();
                                        let hover_id = item.id.clone();
                                        let selected = item.id == selected_id;
                                        rsx! {
                                            div {
                                                id: "ui-command-item-{item.id}",
                                                class: if selected { "ui-command-menu-item ui-command-menu-item--selected" } else { "ui-command-menu-item" },
                                                role: "option",
                                                "aria-selected": if selected { "true" } else { "false" },
                                                "data-active": if selected { "true" } else { "false" },
                                                onclick: move |_| {
                                                    if let Some(handler) = &on_select { handler.call(select_id.clone()); }
                                                },
                                                onmouseenter: move |_| {
                                                    if let Some(handler) = &on_selection_change { handler.call(hover_id.clone()); }
                                                },
                                                if !item.icon_path.is_empty() {
                                                    span { class: "ui-command-menu-item-icon", "aria-hidden": "true",
                                                        svg {
                                                            view_box: "0 0 24 24", width: "16", height: "16",
                                                            fill: "none", stroke: "currentColor", stroke_width: "2",
                                                            stroke_linecap: "round", stroke_linejoin: "round",
                                                            path { d: "{item.icon_path}" }
                                                        }
                                                    }
                                                }
                                                span { class: "ui-command-menu-item-body",
                                                    strong { "{item.label}" }
                                                    span { "{item.description}" }
                                                }
                                                if !item.shortcut.is_empty() {
                                                    kbd { class: "ui-command-menu-item-shortcut", "{item.shortcut}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    p { class: "ui-command-menu-empty", "{empty_text}" }
                }
            }
        }
    }
}

fn step_selection(ids: &[String], current: &str, delta: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let index = ids
        .iter()
        .position(|candidate| candidate == current)
        .map(|index| index as i32)
        .unwrap_or(if delta >= 0 { -1 } else { ids.len() as i32 });
    let len = ids.len() as i32;
    ids.get((((index + delta) % len + len) % len) as usize)
        .cloned()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TourPlacement {
    #[default]
    Bottom,
    Top,
    Center,
}

impl TourPlacement {
    fn class_suffix(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Center => "center",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TourStep {
    pub id: String,
    pub target_id: String,
    pub title: String,
    pub body: String,
    pub placement: TourPlacement,
}

impl TourStep {
    pub fn new(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            target_id: String::new(),
            title: title.into(),
            body: body.into(),
            placement: TourPlacement::Bottom,
        }
    }

    pub fn with_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = target_id.into();
        self
    }

    pub fn with_placement(mut self, placement: TourPlacement) -> Self {
        self.placement = placement;
        self
    }
}

#[cfg(target_arch = "wasm32")]
fn measure_spotlight(overlay_id: &str, target_id: &str, padding: f32) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(overlay) = document.get_element_by_id(overlay_id) else {
        return;
    };
    let Some(target) = document.get_element_by_id(target_id) else {
        let _ = overlay.remove_attribute("data-anchored");
        return;
    };
    let rect = target.get_bounding_client_rect();
    let padding = if padding.is_finite() {
        padding.clamp(0.0, 64.0) as f64
    } else {
        0.0
    };
    let style = format!(
        "--ui-tour-x:{}px;--ui-tour-y:{}px;--ui-tour-w:{}px;--ui-tour-h:{}px",
        rect.left() - padding,
        rect.top() - padding,
        rect.width() + 2.0 * padding,
        rect.height() + 2.0 * padding,
    );
    let _ = overlay.set_attribute("style", &style);
    let _ = overlay.set_attribute("data-anchored", "true");
    target.scroll_into_view_with_bool(false);
}

#[component]
pub fn Tour(
    id: String,
    open: bool,
    steps: Vec<TourStep>,
    #[props(default)] active: usize,
    on_change: EventHandler<usize>,
    on_dismiss: EventHandler<()>,
    #[props(default = "Next".to_string())] next_label: String,
    #[props(default = "Back".to_string())] back_label: String,
    #[props(default = "Done".to_string())] done_label: String,
    #[props(default = "Skip tour".to_string())] skip_label: String,
) -> Element {
    let focus_return = use_hook(FocusReturn::default);
    let restore_on_drop = focus_return.clone();
    use_drop(move || restore_on_drop.restore());

    let count = steps.len();
    let active = active.min(count.saturating_sub(1));
    let _target_for_measure = steps
        .get(active)
        .map(|step| step.target_id.clone())
        .unwrap_or_default();
    let overlay_id = format!("{id}-overlay");
    let _overlay_for_measure = overlay_id.clone();
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        if open && !_target_for_measure.is_empty() {
            measure_spotlight(&_overlay_for_measure, &_target_for_measure, 8.0);
        }
    });

    if !open || count == 0 {
        return rsx! {};
    }

    let step = &steps[active];
    let placement = if step.target_id.is_empty() {
        TourPlacement::Center
    } else {
        step.placement
    };
    let panel_id = format!("{id}-panel");
    let title_id = format!("{id}-title");
    let body_id = format!("{id}-body");
    let panel_class = format!("ui-tour-panel ui-tour-panel--{}", placement.class_suffix());
    let is_first = active == 0;
    let is_last = active + 1 == count;
    // Dioxus consumes `key` during RSX expansion, so rustc does not observe the
    // local read even though it controls virtual-DOM identity.
    let _step_key = step.id.clone();
    let counter = format!("Step {} of {count}", active + 1);
    let _panel_for_key = panel_id.clone();
    let restore_key = focus_return.clone();
    let restore_scrim = focus_return.clone();
    let restore_skip = focus_return.clone();
    let restore_done = focus_return.clone();
    let capture_mount = focus_return.clone();

    rsx! {
        div { class: "ui-tour",
            div {
                id: "{overlay_id}",
                class: "ui-spotlight-overlay",
                onclick: move |_| {
                    restore_scrim.restore();
                    on_dismiss.call(());
                },
                div { class: "ui-spotlight-cutout", "aria-hidden": "true" }
                div {
                    key: _step_key,
                    id: "{panel_id}",
                    class: "{panel_class}",
                    role: "dialog",
                    "aria-modal": "true",
                    "aria-labelledby": "{title_id}",
                    "aria-describedby": "{body_id}",
                    tabindex: "-1",
                    onclick: |event| event.stop_propagation(),
                    onmounted: move |event| {
                        capture_mount.capture();
                        spawn(async move { let _ = event.set_focus(true).await; });
                    },
                    onkeydown: move |event| match event.key() {
                        Key::Escape => {
                            event.prevent_default();
                            restore_key.restore();
                            on_dismiss.call(());
                        }
                        Key::Tab => {
                            #[cfg(target_arch = "wasm32")]
                            if trap_focus(
                                &_panel_for_key,
                                event.modifiers().contains(Modifiers::SHIFT),
                            ) {
                                event.prevent_default();
                            }
                        }
                        _ => {}
                    },
                    p { class: "ui-tour-counter", "{counter}" }
                    h3 { id: "{title_id}", class: "ui-tour-title", "{step.title}" }
                    p { id: "{body_id}", class: "ui-tour-body", "{step.body}" }
                    div { class: "ui-tour-actions",
                        button {
                            class: "ui-button ui-button--ghost ui-tour-skip",
                            r#type: "button",
                            onclick: move |_| {
                                restore_skip.restore();
                                on_dismiss.call(());
                            },
                            "{skip_label}"
                        }
                        div { class: "ui-tour-steps-nav",
                            if !is_first {
                                button {
                                    class: "ui-button ui-button--secondary",
                                    r#type: "button",
                                    onclick: move |_| on_change.call(active.saturating_sub(1)),
                                    "{back_label}"
                                }
                            }
                            button {
                                class: "ui-button ui-button--primary",
                                r#type: "button",
                                onclick: move |_| {
                                    if is_last {
                                        restore_done.restore();
                                        on_dismiss.call(());
                                    } else {
                                        on_change.call(active + 1);
                                    }
                                },
                                if is_last { "{done_label}" } else { "{next_label}" }
                            }
                        }
                    }
                    div { class: "ui-tour-progress", "aria-hidden": "true",
                        for index in 0..count {
                            span { class: if index == active { "ui-tour-dot ui-tour-dot--active" } else { "ui-tour-dot" } }
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
    fn selection_wraps_in_both_directions() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(step_selection(&ids, "c", 1).as_deref(), Some("a"));
        assert_eq!(step_selection(&ids, "a", -1).as_deref(), Some("c"));
    }

    #[test]
    fn overlay_source_has_no_dynamic_script_evaluation() {
        let source = include_str!("csp_overlays.rs");
        assert!(!source.contains(concat!("document", "::eval")));
        assert!(!source.contains(concat!("js_sys::", "Function")));
    }
}
