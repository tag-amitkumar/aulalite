// crates/design-system/src/progress_bar.rs
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum ProgressVariant {
    #[default]
    Primary,
    Success,
    Warning,
    Danger,
}

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    /// 0.0..=1.0
    pub value: f32,
    #[props(default)]
    pub label: Option<String>,
    #[props(default)]
    pub variant: ProgressVariant,
    #[props(default)]
    pub indeterminate: bool,
    #[props(default)]
    pub aria_label: Option<String>,
}

#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    let pct = (props.value.clamp(0.0, 1.0) * 100.0).round() as i32;

    let mut class = String::from("ds-progress-bar");
    match props.variant {
        ProgressVariant::Primary => {}
        ProgressVariant::Success => class.push_str(" ds-progress-bar--success"),
        ProgressVariant::Warning => class.push_str(" ds-progress-bar--warning"),
        ProgressVariant::Danger => class.push_str(" ds-progress-bar--danger"),
    }
    if props.indeterminate {
        class.push_str(" ds-progress-bar--indeterminate");
    }

    // When indeterminate the CSS animation drives the fill; otherwise width
    // is set explicitly.
    let fill_style = if props.indeterminate {
        String::from("width: 0%")
    } else {
        format!("width: {pct}%")
    };

    let aria_label = props
        .aria_label
        .clone()
        .unwrap_or_else(|| "Progress".to_string());

    // For determinate bars expose ARIA value attributes; for indeterminate
    // omit aria-valuenow to signal the unknown state.
    let aria_valuenow: Option<String> = if props.indeterminate {
        None
    } else {
        Some(pct.to_string())
    };

    rsx! {
        div {
            class: "{class}",
            role: "progressbar",
            "aria-label": "{aria_label}",
            "aria-valuemin": "0",
            "aria-valuemax": "100",
            "aria-valuenow": aria_valuenow,
            div { class: "ds-progress-bar-fill", style: "{fill_style}" }
            if let Some(label) = &props.label {
                span { class: "ds-progress-bar-label", "{label}" }
            } else if !props.indeterminate {
                span { class: "ds-progress-bar-label", "{pct}%" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_bar_renders_default_variant() {
        fn app() -> Element {
            rsx! { ProgressBar { value: 0.5 } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-progress-bar"),
            "ds-progress-bar class missing: {html}"
        );
    }

    #[test]
    fn progress_bar_renders_success_variant() {
        fn app() -> Element {
            rsx! { ProgressBar { value: 0.5, variant: ProgressVariant::Success } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-progress-bar--success"),
            "success class missing: {html}"
        );
    }

    #[test]
    fn progress_bar_indeterminate_adds_class() {
        fn app() -> Element {
            rsx! { ProgressBar { value: 0.0, indeterminate: true } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-progress-bar--indeterminate"),
            "indeterminate class missing: {html}"
        );
    }
}
