// crates/features-courses/src/course_builder.rs
use design_system::kinetics_ui::{SortableItem, SortableList};
use design_system::{Button, ButtonVariant, Card, EmptyState, EmptyStateVariant};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct RenamableTitleProps {
    pub value: String,
    pub on_commit: EventHandler<String>,
}

/// Click-to-edit title. Renders a button by default; on click swaps to
/// a text input that commits on Enter / blur and cancels on Escape.
/// Empty / whitespace-only / unchanged values are treated as cancels.
#[component]
pub fn RenamableTitle(props: RenamableTitleProps) -> Element {
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(|| props.value.clone());
    let original = props.value.clone();
    let on_commit = props.on_commit;

    if *editing.read() {
        let original_for_keys = original.clone();
        let original_for_blur = original.clone();
        rsx! {
            input {
                class: "ds-input renamable-title-input",
                r#type: "text",
                value: "{draft}",
                autofocus: true,
                onmounted: move |e: Event<MountedData>| {
                    spawn(async move {
                        let _ = e.set_focus(true).await;
                    });
                },
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    let key = e.key().to_string();
                    if key == "Enter" {
                        e.prevent_default();
                        let new_title = draft.read().trim().to_string();
                        if !new_title.is_empty() && new_title != original_for_keys {
                            on_commit.call(new_title);
                        }
                        editing.set(false);
                    } else if key == "Escape" {
                        editing.set(false);
                    }
                },
                onblur: move |_| {
                    if !*editing.read() {
                        return;
                    }
                    let new_title = draft.read().trim().to_string();
                    if !new_title.is_empty() && new_title != original_for_blur {
                        on_commit.call(new_title);
                    }
                    editing.set(false);
                },
            }
        }
    } else {
        rsx! {
            button {
                class: "renamable-title-display",
                r#type: "button",
                onclick: move |_| {
                    draft.set(original.clone());
                    editing.set(true);
                },
                "{original}"
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct LessonNode {
    pub id: String,
    pub title: String,
    pub r#type: String,
}

#[derive(Clone, PartialEq)]
pub struct ModuleNode {
    pub id: String,
    pub title: String,
    pub lessons: Vec<LessonNode>,
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseBuilderProps {
    pub modules: Vec<ModuleNode>,
    pub on_add_module: EventHandler<()>,
    pub on_add_lesson: EventHandler<String>,
    pub on_lesson_clicked: EventHandler<String>,
    pub on_modules_reordered: EventHandler<Vec<String>>,
    pub on_lessons_reordered: EventHandler<(String, Vec<String>)>,
    /// (module_id, new_title)
    pub on_module_renamed: EventHandler<(String, String)>,
    /// (module_id, lesson_id, new_title)
    pub on_lesson_renamed: EventHandler<(String, String, String)>,
    /// (module_id)
    #[props(default)]
    pub on_module_deleted: EventHandler<String>,
    /// (module_id, lesson_id)
    #[props(default)]
    pub on_lesson_deleted: EventHandler<(String, String)>,
}

/// Pure-function reorder helper.
pub fn move_to_index<T: Clone + PartialEq>(items: &[T], moving: &T, target_idx: usize) -> Vec<T> {
    let mut out: Vec<T> = items.iter().filter(|x| *x != moving).cloned().collect();
    let idx = target_idx.min(out.len());
    out.insert(idx, moving.clone());
    out
}

/// Reorder-mode body: kinetics `SortableList`s (drag + full keyboard a11y)
/// for the modules and for each module's lessons. Pure so it's SSR-testable.
fn reorder_lists(
    modules: &[ModuleNode],
    on_modules_reordered: EventHandler<Vec<String>>,
    on_lessons_reordered: EventHandler<(String, Vec<String>)>,
) -> Element {
    let module_items: Vec<SortableItem> = modules
        .iter()
        .map(|m| {
            SortableItem::new(m.id.clone(), m.title.clone()).with_description(format!(
                "{} lesson{}",
                m.lessons.len(),
                if m.lessons.len() == 1 { "" } else { "s" }
            ))
        })
        .collect();
    rsx! {
        div { class: "builder-reorder",
            SortableList {
                label: "Modules".to_string(),
                items: module_items,
                on_reorder: move |order: Vec<String>| on_modules_reordered.call(order),
            }
            for m in modules.iter().filter(|m| m.lessons.len() > 1) {
                {
                    let module_key = m.id.clone();
                    let module_id = m.id.clone();
                    let lesson_items: Vec<SortableItem> = m
                        .lessons
                        .iter()
                        .map(|l| {
                            SortableItem::new(l.id.clone(), l.title.clone())
                                .with_description(l.r#type.clone())
                        })
                        .collect();
                    rsx! {
                        section { class: "builder-reorder-module", key: "{module_key}",
                            h3 { class: "builder-reorder-module-title", "{m.title}" }
                            SortableList {
                                label: format!("Lessons in {}", m.title),
                                items: lesson_items,
                                on_reorder: move |order: Vec<String>| {
                                    on_lessons_reordered.call((module_id.clone(), order))
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn CourseBuilder(props: CourseBuilderProps) -> Element {
    let mut dragging_module = use_signal(|| None::<String>);
    let mut reorder_mode = use_signal(|| false);
    let on_add_module = props.on_add_module;

    if props.modules.is_empty() {
        return rsx! {
            div { class: "course-builder",
                EmptyState {
                    title: "No modules yet".to_string(),
                    description: "Start by adding the first module — for example 'Week 1'.".to_string(),
                    variant: EmptyStateVariant::Accent,
                    cta: Some(rsx! {
                        Button {
                            label: "Add module".to_string(),
                            variant: ButtonVariant::Primary,
                            on_click: move |_| on_add_module.call(()),
                        }
                    }),
                }
            }
        };
    }

    // Reorder mode swaps the rich editor rows for keyboard-accessible
    // kinetics SortableLists; the toggle round-trips with no data loss since
    // every reorder persists through the same callbacks.
    if *reorder_mode.read() {
        let lists = reorder_lists(
            &props.modules,
            props.on_modules_reordered,
            props.on_lessons_reordered,
        );
        return rsx! {
            div { class: "course-builder",
                div { class: "builder-toolbar",
                    Button {
                        label: "Done reordering".to_string(),
                        variant: ButtonVariant::Primary,
                        on_click: move |_| reorder_mode.set(false),
                    }
                }
                { lists }
            }
        };
    }

    rsx! {
        div { class: "course-builder",
            div { class: "builder-toolbar",
                Button {
                    label: "+ Module".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: move |_| on_add_module.call(()),
                }
                Button {
                    label: "Reorder".to_string(),
                    variant: ButtonVariant::Secondary,
                    on_click: move |_| reorder_mode.set(true),
                }
                span { class: "builder-toolbar-hint",
                    "Drag cards or use Reorder for keyboard-accessible ordering"
                }
            }
            ol { class: "builder-modules",
                for (m_idx, m) in props.modules.iter().enumerate() {
                    {
                        let module_id = m.id.clone();
                        let module_id_for_drop = module_id.clone();
                        let on_modules_reordered = props.on_modules_reordered;
                        let modules_snapshot: Vec<String> =
                            props.modules.iter().map(|m| m.id.clone()).collect();
                        let on_add_lesson = props.on_add_lesson;
                        rsx! {
                            li {
                                key: "{module_id}",
                                class: "builder-module",
                                draggable: "true",
                                ondragstart: move |_| dragging_module.set(Some(module_id.clone())),
                                ondragover: move |evt| evt.prevent_default(),
                                ondrop: move |evt| {
                                    evt.prevent_default();
                                    if let Some(moving) = dragging_module.read().clone() {
                                        let new_order = move_to_index(
                                            &modules_snapshot,
                                            &moving,
                                            m_idx,
                                        );
                                        on_modules_reordered.call(new_order);
                                    }
                                    dragging_module.set(None);
                                },
                                Card {
                                    {
                                        let module_id_for_rename = m.id.clone();
                                        let module_id_for_delete = m.id.clone();
                                        let on_module_renamed = props.on_module_renamed;
                                        let on_module_deleted = props.on_module_deleted;
                                        rsx! {
                                            div { class: "builder-module-header",
                                                h3 {
                                                    class: "builder-module-title",
                                                    RenamableTitle {
                                                        value: m.title.clone(),
                                                        on_commit: move |new_title: String| {
                                                            on_module_renamed.call((module_id_for_rename.clone(), new_title));
                                                        },
                                                    }
                                                }
                                                button {
                                                    class: "module-delete",
                                                    r#type: "button",
                                                    "aria-label": "Delete module",
                                                    title: "Delete module",
                                                    onclick: move |_| {
                                                        on_module_deleted.call(module_id_for_delete.clone());
                                                    },
                                                    "\u{00d7}"
                                                }
                                            }
                                        }
                                    }
                                    ol { class: "builder-lessons",
                                        for l in &m.lessons {
                                            {
                                                let lesson_id_click = l.id.clone();
                                                let lesson_id_for_rename = l.id.clone();
                                                let module_id_for_lesson_rename = m.id.clone();
                                                let on_click = props.on_lesson_clicked;
                                                let on_lesson_renamed = props.on_lesson_renamed;
                                                let lesson_title = l.title.clone();
                                                let lesson_type = l.r#type.clone();
                                                let lesson_id_for_delete = l.id.clone();
                                                let module_id_for_lesson_delete = m.id.clone();
                                                let on_lesson_deleted = props.on_lesson_deleted;
                                                rsx! {
                                                    li {
                                                        key: "{l.id}",
                                                        class: "builder-lesson",
                                                        onclick: move |_| on_click.call(lesson_id_click.clone()),
                                                        span { class: "type-pill", "{lesson_type}" }
                                                        span {
                                                            class: "lesson-title",
                                                            onclick: move |evt: dioxus::events::MouseEvent| evt.stop_propagation(),
                                                            RenamableTitle {
                                                                value: lesson_title.clone(),
                                                                on_commit: move |new_title: String| {
                                                                    on_lesson_renamed.call((
                                                                        module_id_for_lesson_rename.clone(),
                                                                        lesson_id_for_rename.clone(),
                                                                        new_title,
                                                                    ));
                                                                },
                                                            }
                                                        }
                                                        button {
                                                            class: "lesson-delete",
                                                            r#type: "button",
                                                            "aria-label": "Delete lesson",
                                                            title: "Delete lesson",
                                                            onclick: move |evt: dioxus::events::MouseEvent| {
                                                                evt.stop_propagation();
                                                                on_lesson_deleted.call((
                                                                    module_id_for_lesson_delete.clone(),
                                                                    lesson_id_for_delete.clone(),
                                                                ));
                                                            },
                                                            "\u{00d7}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Button {
                                        label: "+ Add lesson".to_string(),
                                        variant: ButtonVariant::Secondary,
                                        on_click: move |_| on_add_lesson.call(module_id_for_drop.clone()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::move_to_index;

    #[test]
    fn move_first_to_last() {
        let v = vec!["a", "b", "c"];
        let out = move_to_index(&v, &"a", 3);
        assert_eq!(out, vec!["b", "c", "a"]);
    }

    #[test]
    fn move_last_to_first() {
        let v = vec!["a", "b", "c"];
        let out = move_to_index(&v, &"c", 0);
        assert_eq!(out, vec!["c", "a", "b"]);
    }

    #[test]
    fn target_index_clamped() {
        let v = vec!["a", "b"];
        let out = move_to_index(&v, &"a", 99);
        assert_eq!(out, vec!["b", "a"]);
    }

    use super::RenamableTitle;
    use dioxus::prelude::*;

    #[test]
    fn renamable_title_renders_value_inside_clickable_button() {
        fn app() -> Element {
            rsx! {
                RenamableTitle {
                    value: "Week 1".to_string(),
                    on_commit: move |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Week 1"),
            "expected title text in render, got: {html}"
        );
        assert!(
            html.contains("renamable-title-display"),
            "expected display class in render, got: {html}"
        );
        assert!(
            !html.contains("renamable-title-input"),
            "input class should not render in the non-editing state, got: {html}"
        );
    }

    use super::{CourseBuilder, LessonNode, ModuleNode};

    #[test]
    fn course_builder_renders_module_title_in_renamable_display() {
        fn app() -> Element {
            rsx! {
                CourseBuilder {
                    modules: vec![ModuleNode {
                        id: "m1".to_string(),
                        title: "Week 1".to_string(),
                        lessons: vec![],
                    }],
                    on_add_module: move |_| {},
                    on_add_lesson: move |_: String| {},
                    on_lesson_clicked: move |_: String| {},
                    on_modules_reordered: move |_: Vec<String>| {},
                    on_lessons_reordered: move |_: (String, Vec<String>)| {},
                    on_module_renamed: move |_: (String, String)| {},
                    on_lesson_renamed: move |_: (String, String, String)| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Week 1"), "missing module title: {html}");
        assert!(
            html.contains("renamable-title-display"),
            "module title should render through RenamableTitle: {html}"
        );
        assert!(
            !html.contains("<h3>Week 1</h3>"),
            "module title should not be a plain h3: {html}"
        );
    }

    #[test]
    fn reorder_lists_render_sortables_for_modules_and_multi_lesson_modules() {
        use super::reorder_lists;
        fn app() -> Element {
            let modules = vec![
                ModuleNode {
                    id: "m1".to_string(),
                    title: "Week 1".to_string(),
                    lessons: vec![
                        LessonNode {
                            id: "l1".to_string(),
                            title: "Limits".to_string(),
                            r#type: "rich_text".to_string(),
                        },
                        LessonNode {
                            id: "l2".to_string(),
                            title: "Derivatives".to_string(),
                            r#type: "rich_text".to_string(),
                        },
                    ],
                },
                ModuleNode {
                    id: "m2".to_string(),
                    title: "Week 2".to_string(),
                    lessons: vec![LessonNode {
                        id: "l3".to_string(),
                        title: "Integrals".to_string(),
                        r#type: "rich_text".to_string(),
                    }],
                },
            ];
            reorder_lists(
                &modules,
                EventHandler::new(|_: Vec<String>| {}),
                EventHandler::new(|_: (String, Vec<String>)| {}),
            )
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // The kinetics sortable renders with its ui-sortable class and keeps
        // keyboard a11y built in.
        assert!(html.contains("ui-sortable"), "sortable missing: {html}");
        assert!(html.contains("Week 1"));
        assert!(html.contains("2 lessons"));
        // Only the multi-lesson module gets a lesson reorder list.
        assert!(html.contains("Lessons in Week 1"), "got: {html}");
        assert!(
            !html.contains("Lessons in Week 2"),
            "single-lesson module should be skipped: {html}"
        );
    }

    #[test]
    fn course_builder_toolbar_offers_reorder_mode() {
        fn app() -> Element {
            rsx! {
                CourseBuilder {
                    modules: vec![ModuleNode {
                        id: "m1".to_string(),
                        title: "Week 1".to_string(),
                        lessons: vec![],
                    }],
                    on_add_module: move |_| {},
                    on_add_lesson: move |_: String| {},
                    on_lesson_clicked: move |_: String| {},
                    on_modules_reordered: move |_: Vec<String>| {},
                    on_lessons_reordered: move |_: (String, Vec<String>)| {},
                    on_module_renamed: move |_: (String, String)| {},
                    on_lesson_renamed: move |_: (String, String, String)| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Reorder"), "reorder toggle missing: {html}");
        assert!(
            html.contains("builder-toolbar-hint"),
            "reorder hint missing from toolbar: {html}"
        );
        // Delete affordances stay quiet icon buttons (the visible "×"), with
        // the action spelled out for assistive tech only.
        assert!(
            html.contains("aria-label=\"Delete module\""),
            "module delete a11y label missing: {html}"
        );
        assert!(
            !html.contains(">Delete<"),
            "delete should not render as a loud text button: {html}"
        );
    }

    #[test]
    fn course_builder_renders_lesson_title_in_renamable_display() {
        fn app() -> Element {
            rsx! {
                CourseBuilder {
                    modules: vec![ModuleNode {
                        id: "m1".to_string(),
                        title: "Week 1".to_string(),
                        lessons: vec![LessonNode {
                            id: "l1".to_string(),
                            title: "Limits".to_string(),
                            r#type: "rich_text".to_string(),
                        }],
                    }],
                    on_add_module: move |_| {},
                    on_add_lesson: move |_: String| {},
                    on_lesson_clicked: move |_: String| {},
                    on_modules_reordered: move |_: Vec<String>| {},
                    on_lessons_reordered: move |_: (String, Vec<String>)| {},
                    on_module_renamed: move |_: (String, String)| {},
                    on_lesson_renamed: move |_: (String, String, String)| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Limits"), "missing lesson title: {html}");
        // Lesson title is now inside a RenamableTitle, which renders its
        // display element with the renamable-title-display class.
        let renamable_count = html.matches("renamable-title-display").count();
        assert!(
            renamable_count >= 2,
            "expected both module and lesson renamable displays, got {renamable_count} in: {html}"
        );
    }
}
