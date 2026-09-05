use dioxus::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIELD_ID: AtomicU64 = AtomicU64::new(1);

/// Accessibility metadata shared with controls rendered inside a [`Field`].
///
/// Keeping this context private to the design system means callers get a
/// correctly associated label and description even when they do not need a
/// stable, application-specific id. Explicit ids still win when supplied.
#[derive(Clone, PartialEq)]
pub(crate) struct FieldContext {
    pub control_id: String,
    pub description_id: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldProps {
    pub children: Element,
    pub label: String,
    #[props(default)]
    pub helper: Option<String>,
    #[props(default)]
    pub error: Option<String>,
    /// HTML id of the wrapped input — used by the <label> for accessibility.
    #[props(default)]
    pub for_id: Option<String>,
    /// When true, the helper/error region renders even if empty (preserves vertical rhythm).
    #[props(default)]
    pub reserve_helper_space: bool,
}

#[component]
pub fn Field(props: FieldProps) -> Element {
    let generated_id =
        use_hook(|| format!("ds-field-{}", NEXT_FIELD_ID.fetch_add(1, Ordering::Relaxed)));
    let control_id = props.for_id.clone().unwrap_or_else(|| generated_id.clone());
    let description_id = format!("{control_id}-description");
    use_context_provider(|| FieldContext {
        control_id: control_id.clone(),
        description_id: description_id.clone(),
    });

    let helper_or_error = props.error.clone().or_else(|| props.helper.clone());
    let show_error = props.error.is_some();
    let helper_class = if show_error {
        "ds-field-helper ds-field-helper--error"
    } else {
        "ds-field-helper"
    };

    rsx! {
        div { class: "ds-field",
            label {
                class: "ds-field-label",
                r#for: "{control_id}",
                "{props.label}"
            }
            {props.children}
            if show_error {
                p {
                    id: "{description_id}",
                    class: "{helper_class}",
                    role: "alert",
                    "aria-live": "assertive",
                    "{helper_or_error.as_deref().unwrap_or_default()}"
                }
            } else if let Some(text) = &helper_or_error {
                p { id: "{description_id}", class: "{helper_class}", "{text}" }
            } else if props.reserve_helper_space {
                p {
                    id: "{description_id}",
                    class: "ds-field-helper",
                    "aria-hidden": "true",
                    ""
                }
            } else {
                // Keep the generated `aria-describedby` target valid when a
                // validation message is introduced on a later render.
                p {
                    id: "{description_id}",
                    class: "ds-field-helper",
                    hidden: true,
                    "aria-hidden": "true",
                    ""
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_renders_label_and_helper() {
        fn app() -> Element {
            rsx! {
                Field {
                    label: "Email".to_string(),
                    helper: "We'll never share your email".to_string(),
                    crate::Input { value: String::new(), input_type: "email".to_string(), on_input: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Email"));
        // dioxus_ssr escapes apostrophes as &#39;, so check the surrounding text.
        assert!(html.contains("never share your email"));
        assert!(html.contains("ds-field-label"));
        assert!(html.contains("ds-field-helper"));
        assert!(!html.contains("for=\"\""), "empty label target: {html}");
        assert!(
            html.contains("for=\"ds-field-") && html.contains("id=\"ds-field-"),
            "generated label/control association missing: {html}"
        );
    }

    #[test]
    fn field_renders_error_overrides_helper() {
        fn app() -> Element {
            rsx! {
                Field {
                    label: "Email".to_string(),
                    helper: "We'll never share your email".to_string(),
                    error: "Email is required".to_string(),
                    crate::Input { value: String::new(), input_type: "email".to_string(), error: true, on_input: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Email is required"));
        assert!(html.contains("ds-field-helper--error"));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("aria-live=\"assertive\""));
        // Helper text is suppressed when an error is present.
        assert!(!html.contains("never share your email"));
    }

    #[test]
    fn explicit_id_labels_the_nested_control() {
        fn app() -> Element {
            rsx! {
                Field {
                    label: "Email".to_string(),
                    for_id: "signup-email".to_string(),
                    crate::Input {
                        value: String::new(),
                        on_input: |_| {},
                    }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("for=\"signup-email\""), "{html}");
        assert!(html.contains("id=\"signup-email\""), "{html}");
        assert!(
            html.contains("aria-describedby=\"signup-email-description\""),
            "{html}"
        );
    }
}
