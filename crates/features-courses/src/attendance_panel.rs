// crates/features-courses/src/attendance_panel.rs
//! Teacher-facing attendance panel for a finished live session.
//!
//! Surfaces the staff-only `GET /v1/sessions/:id/attendance` report as a
//! kinetics `DataTable`. Because the backend returns 403 to non-staff
//! viewers, this panel self-hides for students: a 403 simply renders
//! nothing. Other errors render a quiet inline message; the loaded state
//! renders the table (or an empty-state message when no one attended).

use crate::api::ApiError;
use crate::api::{self, AttendanceDto};
use design_system::kinetics_ui::{DataTable, DataTableColumn, DataTableRow};
use design_system::{Button, ButtonSize, ButtonVariant, SkeletonLine};
use dioxus::prelude::*;

/// Trigger an authenticated download of the staff-only attendance CSV
/// (`GET /v1/sessions/:id/attendance.csv`). The endpoint requires the
/// `Authorization: Bearer <token>` header, so a plain `<a href>` can't reach
/// it — we fetch with the token, wrap the body in a Blob, and click a
/// synthetic anchor with `download` set. Self-contained here (rather than in
/// `api.rs`) so this feature touches only its own files; it reuses the same
/// `base_url` + `id_token` the rest of the API client uses.
///
/// Native (desktop/mobile) builds have no DOM to drive a browser download, so
/// this is a no-op off wasm — the panel/button still compile and render.
#[cfg(target_arch = "wasm32")]
fn download_attendance_csv(cx: &api::ApiContext, session_id: &str) {
    use wasm_bindgen::{closure::Closure, JsCast};
    use wasm_bindgen_futures::JsFuture;

    let base_url = cx.base_url.clone();
    let id_token = cx.id_token.clone();
    let session_id = session_id.to_string();

    wasm_bindgen_futures::spawn_local(async move {
        let url = format!("{base_url}/v1/sessions/{session_id}/attendance.csv");
        let opts = web_sys::RequestInit::new();
        opts.set_method("GET");
        let Ok(req) = web_sys::Request::new_with_str_and_init(&url, &opts) else {
            return;
        };
        if !id_token.is_empty() {
            let _ = req
                .headers()
                .set("authorization", &format!("Bearer {id_token}"));
        }
        api::apply_workspace_header(&req.headers());
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(resp_value) = JsFuture::from(window.fetch_with_request(&req)).await else {
            return;
        };
        let Ok(resp) = resp_value.dyn_into::<web_sys::Response>() else {
            return;
        };
        if !(200..300).contains(&resp.status()) {
            return;
        }
        // Read the body as a Blob so the browser saves bytes verbatim (no
        // string re-encoding) and the Content-Type rides along.
        let Ok(blob_promise) = resp.blob() else {
            return;
        };
        let Ok(blob_value) = JsFuture::from(blob_promise).await else {
            return;
        };
        let Ok(blob) = blob_value.dyn_into::<web_sys::Blob>() else {
            return;
        };
        let Ok(object_url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            return;
        };

        let Some(document) = window.document() else {
            return;
        };
        let Ok(anchor_el) = document.create_element("a") else {
            let _ = web_sys::Url::revoke_object_url(&object_url);
            return;
        };
        let Ok(anchor) = anchor_el.dyn_into::<web_sys::HtmlAnchorElement>() else {
            let _ = web_sys::Url::revoke_object_url(&object_url);
            return;
        };
        anchor.set_href(&object_url);
        anchor.set_download(&format!("attendance-{session_id}.csv"));
        anchor.click();

        // Revoke on the next tick so the click has a chance to start the
        // download before the object URL is freed.
        let revoke_url = object_url.clone();
        let cb = Closure::once_into_js(move || {
            let _ = web_sys::Url::revoke_object_url(&revoke_url);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 0);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn download_attendance_csv(cx: &api::ApiContext, session_id: &str) {
    let cx = cx.clone();
    let session_id = session_id.to_string();
    spawn(async move {
        let path = format!("/v1/sessions/{session_id}/attendance.csv");
        let filename = format!("attendance-{session_id}.csv");
        let _ = api::save_authenticated_download(&cx, &path, &filename, "text/csv").await;
    });
}

#[derive(Props, Clone, PartialEq)]
pub struct AttendancePanelProps {
    pub session_id: String,
}

/// Format an RFC3339 timestamp string into a readable `MMM D, YYYY h:mm AM/PM`
/// form. Falls back to the raw string when parsing fails (so we never blank a
/// real value just because the format is unexpected).
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Format a duration in seconds as `Xm Ys` (e.g. 605 → "10m 5s").
fn format_duration(total_seconds: i32) -> String {
    let secs = total_seconds.max(0);
    let minutes = secs / 60;
    let seconds = secs % 60;
    format!("{minutes}m {seconds}s")
}

/// The displayed "Student" label: display_name, else email, else a short
/// (first-8-char) slice of the user id so the row is never blank.
fn student_label(a: &AttendanceDto) -> String {
    if let Some(name) = a.display_name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = a.email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    let id = &a.user_id;
    let short: String = id.chars().take(8).collect();
    short
}

#[component]
pub fn AttendancePanel(props: AttendancePanelProps) -> Element {
    let cx = api::use_api();
    let session_id = props.session_id.clone();

    let attendance = use_resource({
        let cx = cx.clone();
        let session_id = session_id.clone();
        move || {
            let cx = cx.clone();
            let session_id = session_id.clone();
            async move { api::list_session_attendance(&cx, &session_id).await }
        }
    });

    // Click handler for the Export CSV affordance. Cloned per render so it can
    // be moved into the Button's `on_click` in either loaded state.
    let on_export = {
        let cx = cx.clone();
        let session_id = props.session_id.clone();
        move |_| download_attendance_csv(&cx, &session_id)
    };

    match &*attendance.read_unchecked() {
        // Loaded + empty. Staff can still export (an empty report is valid).
        Some(Ok(items)) if items.is_empty() => rsx! {
            section { class: "attendance-panel",
                div { class: "attendance-panel__header",
                    h3 { class: "attendance-panel__title", "Attendance" }
                    Button {
                        label: "Export CSV".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ButtonSize::Sm,
                        on_click: on_export,
                    }
                }
                p { class: "muted", "No attendance recorded yet." }
            }
        },
        // Loaded with rows → DataTable.
        Some(Ok(items)) => {
            let columns = vec![
                DataTableColumn::new("student", "Student"),
                DataTableColumn::new("joined", "Joined"),
                DataTableColumn::new("left", "Left"),
                DataTableColumn::new("time", "Time"),
                DataTableColumn::new("reconnects", "Reconnects"),
            ];
            let rows = items
                .iter()
                .map(|a| {
                    let student = student_label(a);
                    let joined = format_ts(&a.first_joined_at);
                    let left = a
                        .last_left_at
                        .as_deref()
                        .map(format_ts)
                        .unwrap_or_else(|| "\u{2014}".to_string());
                    let time = format_duration(a.total_seconds);
                    let reconnects = a.reconnect_count.to_string();
                    // Stable row id = user_id so Dioxus diffs rows correctly.
                    DataTableRow::new(
                        a.user_id.clone(),
                        vec![student, joined, left, time, reconnects],
                    )
                })
                .collect::<Vec<_>>();
            rsx! {
                section { class: "attendance-panel",
                    div { class: "attendance-panel__header",
                        h3 { class: "attendance-panel__title", "Attendance" }
                        Button {
                            label: "Export CSV".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_click: on_export,
                        }
                    }
                    DataTable { columns, rows, caption: "Attendance" }
                }
            }
        }
        // 403 → non-staff viewer; self-hide entirely.
        Some(Err(ApiError::Status(403, _))) => rsx! {},
        // Other errors → quiet inline message.
        Some(Err(_)) => rsx! {
            section { class: "attendance-panel",
                h3 { class: "attendance-panel__title", "Attendance" }
                div { class: "system-state system-state--error",
                    "Couldn't load attendance. Please refresh and try again."
                }
            }
        },
        // Loading.
        None => rsx! {
            section { class: "attendance-panel",
                h3 { class: "attendance-panel__title", "Attendance" }
                div { class: "system-state system-state--loading",
                    SkeletonLine { width: "70%".to_string() }
                    SkeletonLine { width: "60%".to_string() }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats_minutes_and_seconds() {
        assert_eq!(format_duration(605), "10m 5s");
        assert_eq!(format_duration(0), "0m 0s");
        assert_eq!(format_duration(59), "0m 59s");
        assert_eq!(format_duration(60), "1m 0s");
    }

    #[test]
    fn duration_clamps_negative_to_zero() {
        assert_eq!(format_duration(-10), "0m 0s");
    }

    #[test]
    fn student_label_prefers_name_then_email_then_short_id() {
        let with_name = AttendanceDto {
            user_id: "0123456789abcdef".into(),
            display_name: Some("Ada Lovelace".into()),
            email: Some("ada@example.com".into()),
            first_joined_at: "2026-05-29T10:00:00Z".into(),
            last_left_at: None,
            total_seconds: 0,
            reconnect_count: 0,
        };
        assert_eq!(student_label(&with_name), "Ada Lovelace");

        let with_email = AttendanceDto {
            display_name: None,
            ..with_name.clone()
        };
        assert_eq!(student_label(&with_email), "ada@example.com");

        let id_only = AttendanceDto {
            display_name: None,
            email: None,
            ..with_name.clone()
        };
        assert_eq!(student_label(&id_only), "01234567");
    }

    #[test]
    fn format_ts_falls_back_on_bad_input() {
        assert_eq!(format_ts("not-a-date"), "not-a-date");
        // A valid RFC3339 timestamp parses (exact rendering is locale of the
        // chrono format string, but it must not equal the raw input).
        assert_ne!(format_ts("2026-05-29T10:00:00Z"), "2026-05-29T10:00:00Z");
    }
}
