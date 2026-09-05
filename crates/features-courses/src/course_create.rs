// crates/features-courses/src/course_create.rs
use crate::error_messages::humanize_error;
use design_system::{
    Button, ButtonVariant, Card, FormError, Input, PageHeader, PageHeaderVariant, Spinner,
};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseCreateProps {
    pub on_created: EventHandler<String>,
    pub create_fn: EventHandler<(String, String, EventHandler<Result<String, String>>)>,
}

#[component]
pub fn CourseCreate(props: CourseCreateProps) -> Element {
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);
    let on_created = props.on_created;
    let create_fn = props.create_fn;

    let mut do_submit = move || {
        let t = title.read().clone();
        if t.trim().is_empty() {
            error.set(Some("Title is required".into()));
            return;
        }
        submitting.set(true);
        error.set(None);
        let oc = on_created;
        let mut error = error;
        let mut submitting = submitting;
        let inner: EventHandler<Result<String, String>> =
            EventHandler::new(move |res: Result<String, String>| match res {
                Ok(slug) => {
                    submitting.set(false);
                    oc.call(slug);
                }
                Err(msg) => {
                    submitting.set(false);
                    error.set(Some(humanize_error(&msg).to_string()));
                }
            });
        create_fn.call((t, description.read().clone(), inner));
    };
    let mut do_submit_btn = do_submit;

    rsx! {
        div { class: "course-create-page motion-page",
            Card {
                PageHeader {
                    kicker: "Catalog".to_string(),
                    title: "New Course".to_string(),
                    variant: PageHeaderVariant::Hero,
                }
                form { onsubmit: move |e| { e.prevent_default(); do_submit_btn(); },
                    div { class: "field",
                        label { "Title" }
                        Input {
                            value: title.read().clone(),
                            placeholder: "e.g. Intro to Calculus".to_string(),
                            input_type: "text".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| title.set(v),
                        }
                    }
                    div { class: "field",
                        label { "Description" }
                        Input {
                            value: description.read().clone(),
                            placeholder: "What's the course about?".to_string(),
                            input_type: "text".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| description.set(v),
                        }
                    }
                    FormError { message: error.read().clone() }
                    div { class: "actions",
                        if *submitting.read() {
                            Spinner {}
                        } else {
                            Button {
                                label: "Create".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| do_submit(),
                            }
                        }
                    }
                }
            }
        }
    }
}
