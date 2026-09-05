// crates/features-courses/src/transcript_view.rs
//! Student transcript body (`/transcript`): consolidated cross-course academic
//! record — per-course weighted total + letter grade (same math as the staff
//! gradebook), lesson progress, graded-work count, and issued certificates.
//! Printable via the browser print dialog so students can export a PDF for
//! parents or administrators.
use crate::api::{self, TranscriptCourseDto};
use design_system::kinetics_ui::{DataTable, DataTableColumn, DataTableRow};
use design_system::{Button, ButtonVariant, EmptyState, SkeletonCard};
use dioxus::prelude::*;

fn format_date(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%-d %b %Y").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// One transcript detail row (accessibility + print mirror of the table).
pub fn transcript_row(course: &TranscriptCourseDto) -> Element {
    let key = course.course_id.clone();
    let title = course.title.clone();
    let joined = format_date(&course.joined_at);
    let progress = if course.lessons_total > 0 {
        format!("{}/{}", course.lessons_completed, course.lessons_total)
    } else {
        "—".to_string()
    };
    let graded = course.graded_released_count.to_string();
    let grade = match (&course.weighted_total, &course.letter_grade) {
        (Some(pct), Some(letter)) => format!("{letter} · {pct:.1}%"),
        _ => "—".to_string(),
    };
    let cert = course
        .certificate
        .as_ref()
        .map(|c| (c.status.clone(), c.credential_id.clone()));
    rsx! {
        div { class: "transcript-row", key: "{key}",
            strong { "{title}" }
            span { class: "transcript-joined", "{joined}" }
            span { class: "transcript-progress", "{progress}" }
            span { class: "transcript-graded", "{graded}" }
            span { class: "transcript-grade", "{grade}" }
            match cert {
                Some((status, _)) => rsx! { span { class: "transcript-cert transcript-cert--{status}", "{status}" } },
                None => rsx! { span { class: "muted", "no certificate" } },
            }
        }
    }
}

/// `/transcript` page body (the route supplies the shell + header).
#[component]
pub fn TranscriptView() -> Element {
    let cx = api::use_api();
    let transcript = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::fetch_my_transcript(&cx).await }
        }
    });

    let snap = transcript.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(data)) => {
            if data.courses.is_empty() {
                rsx! {
                    EmptyState {
                        title: "No courses yet.".to_string(),
                        description: "Enroll in a course — via code, invitation, or the catalog — to start building your transcript.".to_string(),
                    }
                }
            } else {
                let columns = vec![
                    DataTableColumn::new("course", "Course"),
                    DataTableColumn::new("joined", "Joined"),
                    DataTableColumn::new("progress", "Lessons"),
                    DataTableColumn::new("graded", "Graded work"),
                    DataTableColumn::new("grade", "Grade"),
                    DataTableColumn::new("certificate", "Certificate"),
                ];
                let rows: Vec<DataTableRow> = data
                    .courses
                    .iter()
                    .map(|c| {
                        let grade = match (&c.weighted_total, &c.letter_grade) {
                            (Some(pct), Some(letter)) => format!("{letter} ({pct:.1}%)"),
                            _ => "—".to_string(),
                        };
                        let progress = if c.lessons_total > 0 {
                            format!("{}/{}", c.lessons_completed, c.lessons_total)
                        } else {
                            "—".to_string()
                        };
                        DataTableRow::new(
                            c.course_id.clone(),
                            vec![
                                c.title.clone(),
                                format_date(&c.joined_at),
                                progress,
                                c.graded_released_count.to_string(),
                                grade,
                                c.certificate
                                    .as_ref()
                                    .map(|x| x.status.clone())
                                    .unwrap_or_else(|| "—".to_string()),
                            ],
                        )
                    })
                    .collect();
                rsx! {
                    div { class: "transcript-table",
                        DataTable {
                            columns,
                            rows,
                            caption: "Academic transcript".to_string(),
                        }
                    }
                    div { class: "transcript-rows",
                        for course in &data.courses {
                            { transcript_row(course) }
                        }
                    }
                    p { class: "muted",
                        "Letter grades use the standard scale (A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60). Totals cover released, numerically-graded work only."
                    }
                    div { class: "actions",
                        Button {
                            label: "Print / save as PDF".to_string(),
                            variant: ButtonVariant::Secondary,
                            on_click: move |_| {
                                #[cfg(target_arch = "wasm32")]
                                {
                                    if let Some(w) = web_sys::window() {
                                        let _ = w.print();
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load your transcript: {e}" } },
        None => rsx! { SkeletonCard { height: "220px".to_string() } },
    };
    drop(snap);
    body
}

#[cfg(test)]
mod ssr_tests {
    use super::*;
    use crate::api::{TranscriptCertificateDto, TranscriptResponseDto};

    fn sample() -> TranscriptResponseDto {
        TranscriptResponseDto {
            student_user_id: "u1".into(),
            generated_at: "2026-08-23T00:00:00Z".into(),
            courses: vec![TranscriptCourseDto {
                course_id: "co1".into(),
                slug: "algebra".into(),
                title: "Algebra I".into(),
                status: "published".into(),
                role: "student".into(),
                membership_status: "active".into(),
                joined_at: "2026-06-01T12:00:00Z".into(),
                lessons_total: 12,
                lessons_completed: 10,
                graded_released_count: 3,
                weighted_total: Some(87.5),
                letter_grade: Some("B".into()),
                certificate: Some(TranscriptCertificateDto {
                    credential_id: "AULA-1A2B-3C4D".into(),
                    status: "issued".into(),
                }),
            }],
        }
    }

    #[test]
    fn transcript_row_shows_grade_progress_and_certificate() {
        fn app() -> Element {
            let data = sample();
            transcript_row(&data.courses[0])
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Algebra I"), "title missing: {html}");
        assert!(html.contains("B · 87.5%"), "grade missing: {html}");
        assert!(html.contains("10/12"), "progress missing: {html}");
        assert!(html.contains("issued"), "certificate missing: {html}");
    }
}
