use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct PageHeaderProps {
    pub title: String,
    #[props(default)]
    pub kicker: Option<String>,
    #[props(default)]
    pub subtitle: Option<String>,
    /// Right-aligned actions slot (buttons, links, etc.).
    #[props(default)]
    pub actions: Option<Element>,
    #[props(default)]
    pub variant: PageHeaderVariant,
    #[props(default)]
    pub as_tag: HeadingLevel,
}

#[derive(Clone, PartialEq, Default)]
pub enum PageHeaderVariant {
    #[default]
    Default,
    Hero,
}

#[derive(Clone, PartialEq, Default)]
pub enum HeadingLevel {
    #[default]
    H1,
    H2,
}

#[component]
pub fn PageHeader(props: PageHeaderProps) -> Element {
    let class = match props.variant {
        PageHeaderVariant::Default => "ds-page-header",
        PageHeaderVariant::Hero => "ds-page-header ds-page-header--hero",
    };
    let title = rsx! {
        match props.as_tag {
            HeadingLevel::H1 => rsx! { h1 { class: "ds-page-header-title", "{props.title}" } },
            HeadingLevel::H2 => rsx! { h2 { class: "ds-page-header-title", "{props.title}" } },
        }
    };
    rsx! {
        header { class: "{class}",
            div { class: "ds-page-header-text",
                if let Some(k) = &props.kicker {
                    p { class: "ds-page-header-kicker", "{k}" }
                }
                {title}
                if let Some(s) = &props.subtitle {
                    p { class: "ds-page-header-subtitle", "{s}" }
                }
            }
            if let Some(actions) = &props.actions {
                div { class: "ds-page-header-actions", {actions.clone()} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_header_renders_title_only() {
        fn app() -> Element {
            rsx! { PageHeader { title: "Dashboard".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Dashboard"));
        assert!(html.contains("ds-page-header-title"));
        assert!(html.contains("<header"));
        // No kicker/subtitle/actions when not provided
        assert!(!html.contains("ds-page-header-kicker"));
        assert!(!html.contains("ds-page-header-subtitle"));
        assert!(!html.contains("ds-page-header-actions"));
    }

    #[test]
    fn page_header_renders_kicker_subtitle_actions() {
        fn app() -> Element {
            rsx! {
                PageHeader {
                    title: "Courses".to_string(),
                    kicker: "Catalog".to_string(),
                    subtitle: "All your work in one place".to_string(),
                    actions: rsx! { button { "New" } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Catalog"));
        assert!(html.contains("All your work in one place"));
        assert!(html.contains("ds-page-header-kicker"));
        assert!(html.contains("ds-page-header-subtitle"));
        assert!(html.contains("ds-page-header-actions"));
        assert!(html.contains("<button>New</button>"));
    }

    #[test]
    fn page_header_hero_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                PageHeader {
                    title: "Welcome".to_string(),
                    variant: PageHeaderVariant::Hero,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-page-header--hero"));
    }

    #[test]
    fn page_header_as_h2_renders_h2() {
        fn app() -> Element {
            rsx! { PageHeader { title: "Sub".to_string(), as_tag: HeadingLevel::H2 } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<h2"), "h2 missing: {html}");
    }
}
