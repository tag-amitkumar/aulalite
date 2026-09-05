//! Certificate surfaces (learning-suite Cycle 5).
//!
//! - `CourseCertificatesTab` — on the course detail page. Staff see the
//!   eligibility/issue/revoke table; students (the staff endpoint 403s) fall
//!   back to their own certificates for this course.
//! - `MyCertificates` — the student's `/certificates` page: each issued
//!   certificate renders the kinetics `CertificateCard` with a copyable
//!   public verify link and a print affordance (browser print-to-PDF; the
//!   print stylesheet isolates the card).
//! - `CertificateVerifyView` — the PUBLIC `/verify/:credential_id` body.

use crate::api::{self, ApiError, CertificateDto};
use design_system::kinetics_ui::{CertificateCard, DataTable, DataTableColumn, DataTableRow};
use design_system::{Button, ButtonVariant, EmptyState, SkeletonCard};
use dioxus::prelude::*;

/// "12 June 2026" from an RFC3339 timestamp; raw string when parsing fails.
fn format_date(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%-d %B %Y").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// The public verify URL for a credential, built from the current origin on
/// wasm and a placeholder origin in SSR/tests.
fn verify_url(credential_id: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default();
        format!("{origin}/verify/{credential_id}")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("/verify/{credential_id}")
    }
}

/// One issued certificate: the kinetics card plus verify-link + print actions.
/// Pure/presentational so it's SSR-testable.
pub fn issued_certificate(cert: &CertificateDto, on_print: EventHandler<()>) -> Element {
    let recipient = cert
        .recipient_name
        .clone()
        .unwrap_or_else(|| cert.student_email.clone());
    let course = cert
        .course_title
        .clone()
        .unwrap_or_else(|| "Course".to_string());
    let date = cert
        .issued_at
        .as_deref()
        .map(format_date)
        .unwrap_or_default();
    let credential = cert.credential_id.clone().unwrap_or_default();
    let link = verify_url(&credential);
    rsx! {
        div { class: "certificate-item",
            div { class: "certificate-print-area",
                CertificateCard {
                    recipient,
                    course,
                    date,
                    issuer: "AulaLite Academy".to_string(),
                    credential_id: credential.clone(),
                }
            }
            div { class: "certificate-actions",
                a { class: "certificate-verify-link", href: "{link}", target: "_blank",
                    "Verify: {credential}"
                }
                Button {
                    label: "Print / save as PDF".to_string(),
                    variant: ButtonVariant::Secondary,
                    on_click: move |_| on_print.call(()),
                }
            }
        }
    }
}

/// Student `/certificates` page body.
#[component]
pub fn MyCertificates() -> Element {
    let cx = api::use_api();
    let certs = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::get_my_certificates(&cx).await }
        }
    });

    let print = move |_: ()| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(w) = web_sys::window() {
                let _ = w.print();
            }
        }
    };

    let snap = certs.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) => {
            let issued: Vec<&CertificateDto> =
                rows.iter().filter(|c| c.status == "issued").collect();
            let eligible_count = rows.iter().filter(|c| c.status == "eligible").count();
            if issued.is_empty() && eligible_count == 0 {
                rsx! {
                    EmptyState {
                        title: "No certificates yet.".to_string(),
                        description: "Finish a course — every lesson and graded quiz — and your teacher can issue your certificate.".to_string(),
                    }
                }
            } else {
                rsx! {
                    if eligible_count > 0 {
                        p { class: "muted",
                            "{eligible_count} course(s) completed — awaiting teacher approval."
                        }
                    }
                    div { class: "certificates-list",
                        for cert in issued {
                            { issued_certificate(cert, EventHandler::new(print)) }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load certificates: {e}" } },
        None => rsx! { SkeletonCard { height: "260px".to_string() } },
    };
    drop(snap);
    body
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseCertificatesTabProps {
    pub course_id: String,
}

/// Course-detail Certificates tab: staff management table, with a student
/// fallback when the staff endpoint is forbidden.
#[component]
pub fn CourseCertificatesTab(props: CourseCertificatesTabProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let list = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::get_course_certificates(&cx, &course_id).await }
        }
    });

    let snap = list.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) if rows.is_empty() => rsx! {
            EmptyState {
                title: "No completions yet.".to_string(),
                description: "Students appear here once they finish every lesson and graded quiz; you approve and issue each certificate.".to_string(),
            }
        },
        Some(Ok(rows)) => staff_table(&course_id, rows, cx.clone(), move || {
            let mut list = list;
            list.restart()
        }),
        // Students aren't course staff: show their own certificates for this course.
        Some(Err(ApiError::Status(403, _))) => rsx! {
            StudentCourseCertificates { course_id: course_id.clone() }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load certificates: {e}" } },
        None => rsx! { SkeletonCard { height: "200px".to_string() } },
    };
    drop(snap);
    body
}

/// Staff management table with Issue/Revoke actions per row.
fn staff_table(
    course_id: &str,
    rows: &[CertificateDto],
    cx: api::ApiContext,
    refresh: impl Fn() + Clone + 'static,
) -> Element {
    let columns = vec![
        DataTableColumn::new("student", "Student"),
        DataTableColumn::new("status", "Status"),
        DataTableColumn::new("credential", "Credential"),
        DataTableColumn::new("issued", "Issued"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|c| {
            let student = c
                .student_display_name
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| c.student_email.clone());
            DataTableRow::new(
                c.id.clone(),
                vec![
                    student,
                    c.status.clone(),
                    c.credential_id.clone().unwrap_or_else(|| "—".to_string()),
                    c.issued_at
                        .as_deref()
                        .map(format_date)
                        .unwrap_or_else(|| "—".to_string()),
                ],
            )
        })
        .collect();

    let actions = rows.iter().map(|c| {
        let cx = cx.clone();
        let course_id = course_id.to_string();
        let user_id = c.user_id.clone();
        let status = c.status.clone();
        let student = c
            .student_display_name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| c.student_email.clone());
        let refresh = refresh.clone();
        rsx! {
            div { class: "certificate-admin-row", key: "{c.id}",
                span { class: "certificate-admin-student", "{student}" }
                if status == "issued" {
                    Button {
                        label: "Revoke".to_string(),
                        variant: ButtonVariant::Ghost,
                        on_click: move |_| {
                            let cx = cx.clone();
                            let course_id = course_id.clone();
                            let user_id = user_id.clone();
                            let refresh = refresh.clone();
                            spawn(async move {
                                if api::revoke_certificate(&cx, &course_id, &user_id).await.is_ok() {
                                    refresh();
                                }
                            });
                        },
                    }
                } else {
                    Button {
                        label: if status == "revoked" { "Re-issue".to_string() } else { "Issue".to_string() },
                        variant: ButtonVariant::Primary,
                        on_click: move |_| {
                            let cx = cx.clone();
                            let course_id = course_id.clone();
                            let user_id = user_id.clone();
                            let refresh = refresh.clone();
                            spawn(async move {
                                if api::issue_certificate(&cx, &course_id, &user_id).await.is_ok() {
                                    refresh();
                                }
                            });
                        },
                    }
                }
            }
        }
    });

    rsx! {
        section { class: "course-certificates-admin",
            DataTable {
                columns,
                rows: table_rows,
                caption: "Course certificates".to_string(),
            }
            div { class: "certificate-admin-actions",
                h3 { class: "certificate-admin-actions-title", "Actions" }
                { actions }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct StudentCourseCertificatesProps {
    course_id: String,
}

/// Student fallback inside the course tab: own certificates for this course.
#[component]
fn StudentCourseCertificates(props: StudentCourseCertificatesProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let certs = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::get_my_certificates(&cx).await }
        }
    });
    let print = move |_: ()| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(w) = web_sys::window() {
                let _ = w.print();
            }
        }
    };

    let snap = certs.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) => {
            let mine: Vec<&CertificateDto> =
                rows.iter().filter(|c| c.course_id == course_id).collect();
            match mine.iter().find(|c| c.status == "issued") {
                Some(cert) => rsx! {
                    div { class: "certificates-list",
                        { issued_certificate(cert, EventHandler::new(print)) }
                    }
                },
                None if mine.iter().any(|c| c.status == "eligible") => rsx! {
                    p { class: "muted",
                        "You've completed this course — your certificate is awaiting teacher approval."
                    }
                },
                None => rsx! {
                    p { class: "muted",
                        "Complete every lesson and graded quiz to earn this course's certificate."
                    }
                },
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load certificates: {e}" } },
        None => rsx! { SkeletonCard { height: "160px".to_string() } },
    };
    drop(snap);
    body
}

#[derive(Props, Clone, PartialEq)]
pub struct CertificateVerifyViewProps {
    pub credential_id: String,
}

/// PUBLIC verification body: validity banner + the certificate render.
#[component]
pub fn CertificateVerifyView(props: CertificateVerifyViewProps) -> Element {
    let cx = api::use_api();
    let credential_id = props.credential_id.clone();
    let result = use_resource({
        let cx = cx.clone();
        let credential_id = credential_id.clone();
        move || {
            let cx = cx.clone();
            let credential_id = credential_id.clone();
            async move { api::verify_certificate(&cx, &credential_id).await }
        }
    });

    let snap = result.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(v)) if v.status == "issued" => {
            let recipient = v.recipient_name.clone().unwrap_or_default();
            let course = v.course_title.clone().unwrap_or_default();
            let date = v.issued_at.as_deref().map(format_date).unwrap_or_default();
            rsx! {
                p { class: "certificate-verify-status certificate-verify-status--valid",
                    "✓ Valid certificate"
                }
                CertificateCard {
                    recipient,
                    course,
                    date,
                    issuer: "AulaLite Academy".to_string(),
                    credential_id: v.credential_id.clone(),
                }
            }
        }
        Some(Ok(v)) => rsx! {
            p { class: "certificate-verify-status certificate-verify-status--revoked",
                "✕ This certificate ({v.credential_id}) has been revoked."
            }
        },
        Some(Err(ApiError::Status(404, _))) => rsx! {
            p { class: "certificate-verify-status certificate-verify-status--revoked",
                "✕ No certificate found for this credential ID."
            }
            p { class: "muted",
                "Double-check the ID — it looks like AULA-XXXX-XXXX and is printed on the certificate."
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Verification failed: {e}" } },
        None => rsx! { SkeletonCard { height: "260px".to_string() } },
    };
    drop(snap);
    rsx! {
        div { class: "certificate-verify-page",
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    fn cert(status: &str) -> CertificateDto {
        CertificateDto {
            id: "c1".into(),
            course_id: "co1".into(),
            user_id: "u1".into(),
            credential_id: Some("AULA-1A2B-3C4D".into()),
            status: status.into(),
            recipient_name: Some("Sam Student".into()),
            course_title: Some("Algebra".into()),
            student_display_name: Some("Sam Student".into()),
            student_email: "sam@x.test".into(),
            issued_at: Some("2026-06-10T12:00:00Z".into()),
            created_at: "2026-06-01T12:00:00Z".into(),
        }
    }

    #[test]
    fn issued_certificate_renders_card_link_and_print() {
        fn app() -> Element {
            issued_certificate(&cert("issued"), EventHandler::new(|_| {}))
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-certificate"), "card missing: {html}");
        assert!(html.contains("Sam Student"));
        assert!(html.contains("Algebra"));
        assert!(html.contains("10 June 2026"), "date missing: {html}");
        assert!(html.contains("AULA-1A2B-3C4D"));
        assert!(
            html.contains("/verify/AULA-1A2B-3C4D"),
            "verify link missing: {html}"
        );
        assert!(html.contains("Print / save as PDF"));
    }

    #[test]
    fn staff_table_offers_issue_revoke_per_status() {
        fn app() -> Element {
            let rows = vec![cert("eligible"), cert("issued"), cert("revoked")];
            staff_table(
                "co1",
                &rows,
                crate::api::ApiContext {
                    base_url: String::new(),
                    id_token: String::new(),
                },
                || {},
            )
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "table missing: {html}");
        assert!(html.contains("Issue"), "issue button missing: {html}");
        assert!(html.contains("Revoke"), "revoke button missing: {html}");
        assert!(html.contains("Re-issue"), "re-issue button missing: {html}");
    }

    #[test]
    fn format_date_is_human() {
        assert_eq!(format_date("2026-06-10T12:00:00Z"), "10 June 2026");
        assert_eq!(format_date("garbage"), "garbage");
    }
}
