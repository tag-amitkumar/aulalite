// crates/features-courses/src/submission_view.rs
//! Read-only single-submission display. Hides grade fields when
//! released_at is null and viewer is the student (server already filters).

use crate::api::{self, SubmissionDto};
use dioxus::prelude::*;

/// Render a download link for a single file-asset attachment. Resolves a fresh
/// presigned GET URL on mount (mirrors `FileAssetImage`) so we never cache the
/// short-lived URL. The anchor uses `download` + `target=_blank` and
/// `rel="external"` so the shell's same-origin SPA interceptor ignores it.
///
/// The file-asset URL endpoint does not return a filename, so we label the link
/// "Attachment" plus a short id for disambiguation.
#[component]
pub fn FileAssetDownloadLink(asset_id: String) -> Element {
    let cx = api::use_api();
    let short_id: String = asset_id.chars().take(8).collect();
    let resource_asset_id = asset_id.clone();
    let url_resource = use_resource(move || {
        let cx = cx.clone();
        let asset_id = resource_asset_id.clone();
        async move { api::file_asset_get_url(&cx, &asset_id).await.map(|r| r.url) }
    });

    match &*url_resource.read_unchecked() {
        Some(Ok(url)) => rsx! {
            a {
                class: "attachment-link",
                href: "{url}",
                download: true,
                target: "_blank",
                rel: "external",
                "Attachment ({short_id})"
            }
        },
        Some(Err(_)) => rsx! {
            span { class: "attachment-error", "Attachment ({short_id}) — unavailable" }
        },
        None => rsx! {
            span { class: "attachment-loading", "Attachment ({short_id}) — …" }
        },
    }
}

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionViewProps {
    pub submission: SubmissionDto,
}

pub fn SubmissionView(props: SubmissionViewProps) -> Element {
    let s = &props.submission;
    rsx! {
        div { class: "submission-view",
            p { class: "status", "Status: {s.status}" }
            if let Some(text) = s.text_answer.as_ref() {
                section { class: "answer",
                    h3 { "Answer" }
                    p { "{text}" }
                }
            }
            if !s.attachment_asset_ids.is_empty() {
                section { class: "attachments",
                    h3 { "Attachments" }
                    ul {
                        for id in s.attachment_asset_ids.iter() {
                            li { key: "{id}",
                                FileAssetDownloadLink { asset_id: id.clone() }
                            }
                        }
                    }
                }
            }
            if s.released_at.is_some() {
                section { class: "grade-block",
                    h3 { "Grade" }
                    if let Some(n) = s.numeric_grade { p { "Numeric: {n}" } }
                    if let Some(l) = s.letter_grade.as_ref() { p { "Letter: {l}" } }
                    if let Some(p) = s.passed { p { "Pass: {p}" } }
                    {
                        match s.applied_late_penalty_percent {
                            Some(p) if p > 0 => rsx! {
                                p { class: "late-penalty",
                                    "Late penalty applied: -{p}% (grade shown is after the penalty)."
                                }
                            },
                            _ => rsx! {},
                        }
                    }
                    if let Some(fb) = s.student_visible_feedback.as_ref() {
                        p { class: "feedback", "Feedback: {fb}" }
                    }
                }
            }
        }
    }
}
