// crates/features-courses/src/start_now_modal.rs
//! Optional "Customize…" modal for the start-now flow. Lets the teacher
//! override title, duration, and recording before posting.

use design_system::{Button, ButtonVariant, Field, Input, Select, SelectOption, Toggle};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct StartNowDraft {
    pub title: String,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct StartNowModalProps {
    pub initial: StartNowDraft,
    pub submitting: bool,
    pub on_submit: EventHandler<StartNowDraft>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn StartNowModal(props: StartNowModalProps) -> Element {
    let mut draft = use_signal(|| props.initial.clone());

    let duration_options = vec![
        SelectOption {
            value: "30".to_string(),
            label: "30 min".to_string(),
        },
        SelectOption {
            value: "45".to_string(),
            label: "45 min".to_string(),
        },
        SelectOption {
            value: "60".to_string(),
            label: "60 min".to_string(),
        },
        SelectOption {
            value: "90".to_string(),
            label: "90 min".to_string(),
        },
    ];

    rsx! {
        div { class: "start-now-modal-backdrop",
            div { class: "start-now-modal",
                role: "dialog",
                aria_modal: "true",
                aria_label: "Start session now",
                h2 { "Start session now" }
                Field { label: "Title".to_string(),
                    Input {
                        value: draft.read().title.clone(),
                        input_type: "text".to_string(),
                        disabled: props.submitting,
                        on_input: move |v| {
                            let mut d = draft.read().clone();
                            d.title = v;
                            draft.set(d);
                        },
                    }
                }
                Field { label: "Duration".to_string(),
                    Select {
                        value: draft.read().duration_minutes.to_string(),
                        options: duration_options.clone(),
                        disabled: props.submitting,
                        on_change: move |v: String| {
                            let mut d = draft.read().clone();
                            if let Ok(n) = v.parse::<i32>() {
                                d.duration_minutes = n;
                                draft.set(d);
                            }
                        },
                    }
                }
                Field { label: "Recording".to_string(),
                    Toggle {
                        checked: draft.read().recording_enabled,
                        disabled: props.submitting,
                        on_change: move |checked: bool| {
                            let mut d = draft.read().clone();
                            d.recording_enabled = checked;
                            draft.set(d);
                        },
                    }
                }
                div { class: "start-now-modal-actions",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Ghost,
                        button_type: "button".to_string(),
                        disabled: props.submitting,
                        on_click: move |_| props.on_cancel.call(()),
                    }
                    Button {
                        label: if props.submitting { "Starting…".to_string() } else { "Start".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: props.submitting,
                        on_click: move |_| props.on_submit.call(draft.read().clone()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> StartNowDraft {
        StartNowDraft {
            title: "Quick session — May 17, 2:32 PM UTC".to_string(),
            duration_minutes: 60,
            recording_enabled: true,
        }
    }

    #[test]
    fn renders_initial_title_and_duration() {
        fn app() -> Element {
            rsx! {
                StartNowModal {
                    initial: super::tests::draft(),
                    submitting: false,
                    on_submit: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Quick session"),
            "title not prefilled: {html}"
        );
        assert!(
            html.contains("Start session now"),
            "modal header missing: {html}"
        );
    }

    #[test]
    fn shows_starting_label_while_submitting() {
        fn app() -> Element {
            rsx! {
                StartNowModal {
                    initial: super::tests::draft(),
                    submitting: true,
                    on_submit: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Starting\u{2026}"),
            "submit-pending label missing: {html}"
        );
    }
}
