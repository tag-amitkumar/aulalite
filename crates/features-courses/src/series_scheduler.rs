// crates/features-courses/src/series_scheduler.rs
use design_system::{
    Button, ButtonVariant, Card, DateTimePicker, Field, FormError, Input, Select, SelectOption,
    Spinner, Toggle,
};
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct SeriesDraft {
    pub title: String,
    pub starts_at_iso: String,
    pub duration_minutes: i32,
    pub frequency: String,
    pub byweekday: Vec<String>,
    pub end_kind: String,
    pub occurrence_count: Option<i32>,
    pub end_until_iso: Option<String>,
    pub recording_enabled: Option<bool>,
}

#[derive(Props, Clone, PartialEq)]
pub struct SeriesSchedulerProps {
    pub initial: SeriesDraft,
    pub preview: Vec<String>,
    pub on_change: EventHandler<SeriesDraft>,
    pub on_submit: EventHandler<SeriesDraft>,
    pub submitting: bool,
    pub error: Option<String>,
}

#[component]
pub fn SeriesScheduler(props: SeriesSchedulerProps) -> Element {
    let mut draft = use_signal(|| props.initial.clone());
    let on_change = props.on_change;

    let weekday_options = [
        ("mon", "Mon"),
        ("tue", "Tue"),
        ("wed", "Wed"),
        ("thu", "Thu"),
        ("fri", "Fri"),
        ("sat", "Sat"),
        ("sun", "Sun"),
    ];

    rsx! {
        Card {
            h2 { "Schedule a live session" }
            Field { label: "Title".to_string(),
                Input {
                    value: draft.read().title.clone(),
                    placeholder: "e.g. Weekly Calc Class".to_string(),
                    input_type: "text".to_string(),
                    disabled: props.submitting,
                    on_input: move |v| {
                        let mut d = draft.read().clone();
                        d.title = v;
                        draft.set(d.clone());
                        on_change.call(d);
                    },
                }
            }
            Field { label: "Starts at".to_string(),
                DateTimePicker {
                    value: draft.read().starts_at_iso.clone(),
                    disabled: props.submitting,
                    on_change: move |v| {
                        let mut d = draft.read().clone();
                        d.starts_at_iso = v;
                        draft.set(d.clone());
                        on_change.call(d);
                    },
                }
            }
            Field { label: "Duration (minutes)".to_string(),
                Input {
                    value: draft.read().duration_minutes.to_string(),
                    placeholder: "60".to_string(),
                    input_type: "number".to_string(),
                    disabled: props.submitting,
                    on_input: move |v: String| {
                        if let Ok(n) = v.parse::<i32>() {
                            let mut d = draft.read().clone();
                            d.duration_minutes = n;
                            draft.set(d.clone());
                            on_change.call(d);
                        }
                    },
                }
            }
            Field { label: "Frequency".to_string(),
                Select {
                    value: draft.read().frequency.clone(),
                    options: vec![
                        SelectOption { value: "none".to_string(), label: "Just once".to_string() },
                        SelectOption { value: "daily".to_string(), label: "Daily".to_string() },
                        SelectOption { value: "weekly".to_string(), label: "Weekly".to_string() },
                        SelectOption { value: "biweekly".to_string(), label: "Every other week".to_string() },
                        SelectOption { value: "monthly".to_string(), label: "Monthly".to_string() },
                    ],
                    on_change: move |v: String| {
                        let mut d = draft.read().clone();
                        d.frequency = v.clone();
                        if !matches!(v.as_str(), "weekly" | "biweekly") {
                            d.byweekday.clear();
                        }
                        draft.set(d.clone());
                        on_change.call(d);
                    },
                }
            }
            if matches!(draft.read().frequency.as_str(), "weekly" | "biweekly") {
                Field { label: "On these days".to_string(),
                    div { class: "weekday-chips",
                        for (key, label) in &weekday_options {
                            {
                                let key_s: String = (*key).into();
                                let key_for_check = key_s.clone();
                                let label_s: String = (*label).into();
                                let active = draft.read().byweekday.contains(&key_s);
                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: if active { "chip chip-active" } else { "chip" },
                                        onclick: move |_| {
                                            let mut d = draft.read().clone();
                                            if d.byweekday.contains(&key_for_check) {
                                                d.byweekday.retain(|x| x != &key_for_check);
                                            } else {
                                                d.byweekday.push(key_for_check.clone());
                                            }
                                            draft.set(d.clone());
                                            on_change.call(d);
                                        },
                                        "{label_s}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Field { label: "Ends".to_string(),
                Select {
                    value: draft.read().end_kind.clone(),
                    options: vec![
                        SelectOption { value: "count".to_string(), label: "After N occurrences".to_string() },
                        SelectOption { value: "until".to_string(), label: "On date".to_string() },
                        SelectOption { value: "open".to_string(), label: "Open-ended (rolling)".to_string() },
                    ],
                    on_change: move |v: String| {
                        let mut d = draft.read().clone();
                        d.end_kind = v.clone();
                        if v != "count" { d.occurrence_count = None; }
                        if v != "until" { d.end_until_iso = None; }
                        draft.set(d.clone());
                        on_change.call(d);
                    },
                }
            }
            if draft.read().end_kind == "count" {
                Field { label: "Occurrence count".to_string(),
                    Input {
                        value: draft.read().occurrence_count.map(|n| n.to_string()).unwrap_or_default(),
                        placeholder: "12".to_string(),
                        input_type: "number".to_string(),
                        disabled: props.submitting,
                        on_input: move |v: String| {
                            let mut d = draft.read().clone();
                            d.occurrence_count = v.parse().ok();
                            draft.set(d.clone());
                            on_change.call(d);
                        },
                    }
                }
            }
            if draft.read().end_kind == "until" {
                Field { label: "End date/time".to_string(),
                    DateTimePicker {
                        value: draft.read().end_until_iso.clone().unwrap_or_default(),
                        disabled: props.submitting,
                        on_change: move |v: String| {
                            let mut d = draft.read().clone();
                            d.end_until_iso = if v.is_empty() { None } else { Some(v) };
                            draft.set(d.clone());
                            on_change.call(d);
                        },
                    }
                }
            }
            div { class: "ds-field",
                Toggle {
                    checked: draft.read().recording_enabled.unwrap_or(true),
                    label: "Record sessions".to_string(),
                    on_change: move |b| {
                        let mut d = draft.read().clone();
                        d.recording_enabled = Some(b);
                        draft.set(d.clone());
                        on_change.call(d);
                    },
                }
            }
            FormError { message: props.error.clone() }
            div { class: "preview",
                h4 { "Preview ({props.preview.len()} occurrences)" }
                ul {
                    for line in &props.preview {
                        li { "{line}" }
                    }
                }
            }
            div { class: "actions",
                if props.submitting {
                    Spinner {}
                } else {
                    Button {
                        label: "Schedule".to_string(),
                        variant: ButtonVariant::Primary,
                        button_type: "submit".to_string(),
                        on_click: move |_| props.on_submit.call(draft.read().clone()),
                    }
                }
            }
        }
    }
}
