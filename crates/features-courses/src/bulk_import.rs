// crates/features-courses/src/bulk_import.rs
//! Staff-only CSV bulk import panel: paste a roster (email, role) or a grade
//! sheet (student, assignment, grade), import, and review a per-row results
//! table. Mounted in the course People/members admin area.
//!
//! Mirrors `announcements.rs` for the API-client-in-module + use_api() + toast
//! pattern, and `submissions_grading_table.rs` for the design-system Table use.
//! Posts JSON `{ "csv": "..." }`; the backend also accepts raw text/csv, but
//! JSON rides the existing `api::fetch_json` path (which sets a JSON content
//! type) with no extra plumbing.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, Table, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror crates/backend/src/handlers/bulk.rs)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct RowResult {
    pub row: usize,
    pub outcome: String,
    pub subject: String,
    pub detail: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct BulkSummary {
    pub total: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub errored: usize,
    pub results: Vec<RowResult>,
}

#[derive(serde::Serialize)]
struct CsvBody<'a> {
    csv: &'a str,
}

/// `POST /v1/courses/{cid}/bulk/enroll` — staff-only roster import.
pub async fn bulk_enroll(
    ctx: &ApiContext,
    course_id: &str,
    csv: &str,
) -> Result<BulkSummary, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/bulk/enroll"),
        Some(&CsvBody { csv }),
    )
    .await
}

/// `POST /v1/courses/{cid}/bulk/grades` — staff-only grade import.
pub async fn bulk_grades(
    ctx: &ApiContext,
    course_id: &str,
    csv: &str,
) -> Result<BulkSummary, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/bulk/grades"),
        Some(&CsvBody { csv }),
    )
    .await
}

// ---------------------------------------------------------------------------
// View helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Roster,
    Grades,
}

fn outcome_tone(outcome: &str) -> BadgeTone {
    match outcome {
        "enrolled" | "created" | "graded" => BadgeTone::Success,
        "invited" => BadgeTone::Info,
        "skipped" => BadgeTone::Neutral,
        "error" => BadgeTone::Danger,
        _ => BadgeTone::Neutral,
    }
}

fn placeholder_for(mode: Mode) -> &'static str {
    match mode {
        Mode::Roster => "email,role\nada@example.com,student\ngrace@example.com,teacher",
        Mode::Grades => {
            "student,assignment,grade\nada@example.com,Homework 1,92\ngrace@example.com,Homework 1,pass"
        }
    }
}

fn help_for(mode: Mode) -> &'static str {
    match mode {
        Mode::Roster => {
            "One row per person: email, role (teacher/ta/student; defaults to student). \
             Existing members are enrolled directly; new emails get an invitation."
        }
        Mode::Grades => {
            "One row per grade: student (email or id), assignment (exact title or id), \
             grade (a number for points, or pass/fail). Up to 1000 rows."
        }
    }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct BulkImportProps {
    pub course_id: String,
}

#[component]
pub fn BulkImport(props: BulkImportProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();

    let mut mode = use_signal(|| Mode::Roster);
    let mut csv = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut summary = use_signal(|| Option::<BulkSummary>::None);
    let mut toast = use_toast_sender();

    let current_mode = *mode.read();

    let on_import = move |_| {
        let api = api.clone();
        let course_id = course_id.clone();
        let text = csv.read().trim().to_string();
        if text.is_empty() {
            toast.push(
                ToastLevel::Warning,
                "Nothing to import",
                "Paste some CSV rows first.",
            );
            return;
        }
        let m = *mode.read();
        submitting.set(true);
        summary.set(None);
        spawn(async move {
            let result = match m {
                Mode::Roster => bulk_enroll(&api, &course_id, &text).await,
                Mode::Grades => bulk_grades(&api, &course_id, &text).await,
            };
            match result {
                Ok(s) => {
                    toast.push(
                        ToastLevel::Success,
                        "Import finished",
                        format!(
                            "{} ok, {} skipped, {} errored",
                            s.succeeded, s.skipped, s.errored
                        ),
                    );
                    summary.set(Some(s));
                }
                Err(e) => {
                    toast.push(ToastLevel::Danger, "Import failed", format!("{e}"));
                }
            }
            submitting.set(false);
        });
    };

    let is_submitting = *submitting.read();
    let summary_snapshot = summary.read().clone();

    rsx! {
        Card {
            div { class: "bulk-import",
                div { class: "bulk-import__header",
                    h2 { "Bulk import" }
                    div { class: "bulk-import__modes",
                        Button {
                            label: "Roster".to_string(),
                            variant: if current_mode == Mode::Roster { ButtonVariant::Primary } else { ButtonVariant::Ghost },
                            button_type: "button".to_string(),
                            disabled: is_submitting,
                            on_click: move |_| {
                                mode.set(Mode::Roster);
                                summary.set(None);
                            },
                        }
                        Button {
                            label: "Grades".to_string(),
                            variant: if current_mode == Mode::Grades { ButtonVariant::Primary } else { ButtonVariant::Ghost },
                            button_type: "button".to_string(),
                            disabled: is_submitting,
                            on_click: move |_| {
                                mode.set(Mode::Grades);
                                summary.set(None);
                            },
                        }
                    }
                }
                p { class: "bulk-import__help muted", "{help_for(current_mode)}" }
                textarea {
                    class: "ds-input bulk-import__textarea",
                    rows: "10",
                    spellcheck: "false",
                    placeholder: "{placeholder_for(current_mode)}",
                    disabled: is_submitting,
                    value: "{csv}",
                    oninput: move |e| csv.set(e.value()),
                }
                div { class: "bulk-import__actions",
                    Button {
                        label: if is_submitting { "Importing…".to_string() } else { "Import".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: is_submitting,
                        on_click: on_import,
                    }
                }

                if let Some(s) = summary_snapshot {
                    div { class: "bulk-import__results",
                        p { class: "bulk-import__counts",
                            "Processed {s.total}: "
                            span { class: "ok", "{s.succeeded} ok" }
                            ", "
                            span { class: "muted", "{s.skipped} skipped" }
                            ", "
                            span { class: "error", "{s.errored} errored" }
                        }
                        Table {
                            head: rsx! {
                                tr {
                                    th { class: "ds-table-th", "Row" }
                                    th { class: "ds-table-th", "Subject" }
                                    th { class: "ds-table-th", "Outcome" }
                                    th { class: "ds-table-th", "Detail" }
                                }
                            },
                            body: rsx! {
                                for r in s.results.iter() {
                                    tr { key: "{r.row}",
                                        td { "{r.row}" }
                                        td { "{r.subject}" }
                                        td {
                                            Badge {
                                                label: r.outcome.clone(),
                                                tone: outcome_tone(&r.outcome),
                                            }
                                        }
                                        td { "{r.detail}" }
                                    }
                                }
                            },
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
    fn outcome_tone_maps_known_outcomes() {
        assert!(matches!(outcome_tone("enrolled"), BadgeTone::Success));
        assert!(matches!(outcome_tone("graded"), BadgeTone::Success));
        assert!(matches!(outcome_tone("invited"), BadgeTone::Info));
        assert!(matches!(outcome_tone("skipped"), BadgeTone::Neutral));
        assert!(matches!(outcome_tone("error"), BadgeTone::Danger));
        assert!(matches!(outcome_tone("???"), BadgeTone::Neutral));
    }

    #[test]
    fn placeholders_differ_per_mode() {
        assert_ne!(placeholder_for(Mode::Roster), placeholder_for(Mode::Grades));
        assert!(placeholder_for(Mode::Roster).contains("role"));
        assert!(placeholder_for(Mode::Grades).contains("assignment"));
    }
}
