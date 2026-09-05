// crates/features-courses/src/gradebook_panel.rs
//! Staff-only weighted gradebook for a course.
//!
//! Renders a matrix (students × published assignments) with a per-student
//! weighted-total column, a category manager (create / delete categories +
//! assign each assignment to a category), and an authenticated "Export CSV"
//! download. The whole panel is STAFF-ONLY: the backend returns 403 to
//! non-staff, so a 403 simply renders nothing and the panel self-hides for
//! students (mirrors `attendance_panel`).
//!
//! API client wrappers live here (not in `api.rs`) and call the publicly
//! re-exported `api::fetch_json`, matching the `announcements` module. The CSV
//! download is the same authenticated-Blob pattern as `attendance_panel`.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Button, ButtonSize, ButtonVariant, Card, EmptyState, Select, SelectOption,
    SkeletonLine, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror crates/backend/src/handlers/gradebook.rs). Uuid serializes as a
// JSON string, so ids decode as `String`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct CategoryDto {
    pub id: String,
    pub course_id: String,
    pub name: String,
    pub weight_percent: i32,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct GradebookAssignmentDto {
    pub id: String,
    pub title: String,
    pub max_points: Option<i32>,
    pub category_id: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct GradeCellDto {
    pub assignment_id: String,
    pub numeric_grade: f64,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct GradebookStudentDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub grades: Vec<GradeCellDto>,
    pub weighted_total: Option<f64>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct GradebookDto {
    pub course_id: String,
    pub categories: Vec<CategoryDto>,
    pub assignments: Vec<GradebookAssignmentDto>,
    pub students: Vec<GradebookStudentDto>,
}

#[derive(serde::Serialize)]
struct CreateCategoryBody<'a> {
    name: &'a str,
    weight_percent: i32,
}

#[derive(serde::Serialize)]
struct SetCategoryBody {
    category_id: Option<String>,
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

/// `GET /v1/courses/{cid}/gradebook` — staff-only matrix.
pub async fn get_gradebook(ctx: &ApiContext, course_id: &str) -> Result<GradebookDto, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/gradebook"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{cid}/grade-categories` — staff-only.
pub async fn create_category(
    ctx: &ApiContext,
    course_id: &str,
    name: &str,
    weight_percent: i32,
) -> Result<CategoryDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/grade-categories"),
        Some(&CreateCategoryBody {
            name,
            weight_percent,
        }),
    )
    .await
}

/// `DELETE /v1/courses/{cid}/grade-categories/{id}` — staff-only.
pub async fn delete_category(ctx: &ApiContext, course_id: &str, id: &str) -> Result<(), ApiError> {
    api::fetch_json::<()>(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/grade-categories/{id}"),
        None::<&()>,
    )
    .await
    .map(|_| ())
}

/// `PUT /v1/courses/{cid}/assignments/{aid}/category` — (un)assign a category.
pub async fn set_assignment_category(
    ctx: &ApiContext,
    course_id: &str,
    assignment_id: &str,
    category_id: Option<String>,
) -> Result<(), ApiError> {
    api::fetch_json::<()>(
        ctx,
        "PUT",
        &format!("/v1/courses/{course_id}/assignments/{assignment_id}/category"),
        Some(&SetCategoryBody { category_id }),
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// CSV download (authenticated Blob — mirrors attendance_panel)
// ---------------------------------------------------------------------------

/// Trigger an authenticated download of the staff-only gradebook CSV
/// (`GET /v1/courses/:cid/gradebook.csv`). The endpoint requires the bearer
/// token, so a plain `<a href>` can't reach it — fetch with the token, wrap the
/// body in a Blob, and click a synthetic anchor. wasm-only (no DOM on native).
#[cfg(target_arch = "wasm32")]
fn download_gradebook_csv(cx: &ApiContext, course_id: &str) {
    use wasm_bindgen::{closure::Closure, JsCast};
    use wasm_bindgen_futures::JsFuture;

    let base_url = cx.base_url.clone();
    let id_token = cx.id_token.clone();
    let course_id = course_id.to_string();

    wasm_bindgen_futures::spawn_local(async move {
        let url = format!("{base_url}/v1/courses/{course_id}/gradebook.csv");
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
        anchor.set_download(&format!("gradebook-{course_id}.csv"));
        anchor.click();

        let revoke_url = object_url.clone();
        let cb = Closure::once_into_js(move || {
            let _ = web_sys::Url::revoke_object_url(&revoke_url);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 0);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn download_gradebook_csv(cx: &ApiContext, course_id: &str) {
    let cx = cx.clone();
    let course_id = course_id.to_string();
    spawn(async move {
        let path = format!("/v1/courses/{course_id}/gradebook.csv");
        let filename = format!("gradebook-{course_id}.csv");
        let _ = api::save_authenticated_download(&cx, &path, &filename, "text/csv").await;
    });
}

// ---------------------------------------------------------------------------
// Rendering helpers (pure → SSR-testable)
// ---------------------------------------------------------------------------

/// Display label for a student: display_name, else email, else a short slice of
/// the user id so the row is never blank.
fn student_label(st: &GradebookStudentDto) -> String {
    if let Some(name) = st.display_name.as_ref().filter(|s| !s.is_empty()) {
        return name.clone();
    }
    if let Some(email) = st.email.as_ref().filter(|s| !s.is_empty()) {
        return email.clone();
    }
    st.user_id.chars().take(8).collect()
}

/// One-decimal grade, or an em-dash for an absent (ungraded) cell.
fn fmt_grade(v: Option<f64>) -> String {
    match v {
        Some(g) => format!("{g:.1}"),
        None => "\u{2014}".to_string(),
    }
}

/// One-decimal percentage for the weighted total, or an em-dash when there is
/// no counted coursework yet.
fn fmt_total(v: Option<f64>) -> String {
    match v {
        Some(t) => format!("{t:.1}%"),
        None => "\u{2014}".to_string(),
    }
}

/// The student's released numeric grade for `assignment_id`, or `None` when the
/// cell is ungraded.
fn cell_grade(st: &GradebookStudentDto, assignment_id: &str) -> Option<f64> {
    st.grades
        .iter()
        .find(|g| g.assignment_id == assignment_id)
        .map(|g| g.numeric_grade)
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct GradebookPanelProps {
    pub course_id: String,
    /// Staff flag from the route dispatcher. The backend is the source of truth
    /// (403 hides the panel); this is just a cheap early-out so students never
    /// fire the staff-only fetch.
    pub is_teacher: bool,
    /// Only Teach-capable staff may change categories and grading policy.
    /// TAs retain gradebook visibility/export without authoring controls.
    #[props(default)]
    pub can_configure: bool,
}

#[component]
pub fn GradebookPanel(props: GradebookPanelProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();
    let is_teacher = props.is_teacher;

    // All hooks run unconditionally (Rules of Hooks). The resource future
    // itself short-circuits for non-staff so students never fire the staff-only
    // fetch; the backend 403 is still the source of truth, and this avoids both
    // the wasted request and a flash of skeleton.
    let mut gradebook = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move {
                if !is_teacher {
                    return Err(ApiError::Status(403, "forbidden".into()));
                }
                get_gradebook(&api, &course_id).await
            }
        }
    });

    let on_export = {
        let api = api.clone();
        let course_id = course_id.clone();
        move |_| download_gradebook_csv(&api, &course_id)
    };

    // Students never see the gradebook — render nothing entirely.
    if !is_teacher {
        return rsx! {};
    }

    let snap = gradebook.read_unchecked();
    let body = match snap.as_ref() {
        // 403 → not staff; self-hide entirely.
        Some(Err(ApiError::Status(403, _))) => return rsx! {},
        Some(Ok(gb)) => {
            let gb = gb.clone();
            rsx! {
                GradebookContent {
                    api: api.clone(),
                    course_id: course_id.clone(),
                    gradebook: gb,
                    can_configure: props.can_configure,
                    on_changed: move |_| gradebook.restart(),
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "system-state system-state--error", "Couldn't load the gradebook: {e}" }
        },
        None => rsx! {
            div { class: "system-state system-state--loading",
                SkeletonLine { width: "80%".to_string() }
                SkeletonLine { width: "70%".to_string() }
                SkeletonLine { width: "60%".to_string() }
            }
        },
    };
    drop(snap);

    rsx! {
        div { class: "gradebook-panel motion-page",
            div { class: "gradebook-panel__header",
                h2 { "Gradebook" }
                Button {
                    label: "Export CSV".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    button_type: "button".to_string(),
                    on_click: on_export,
                }
            }
            { body }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct GradebookContentProps {
    api: ApiContext,
    course_id: String,
    gradebook: GradebookDto,
    can_configure: bool,
    on_changed: EventHandler<()>,
}

#[component]
fn GradebookContent(props: GradebookContentProps) -> Element {
    let gb = props.gradebook.clone();

    rsx! {
        if props.can_configure {
            CategoryManager {
                api: props.api.clone(),
                course_id: props.course_id.clone(),
                categories: gb.categories.clone(),
                assignments: gb.assignments.clone(),
                on_changed: props.on_changed,
            }
        }
        GradeGrid { gradebook: gb }
    }
}

/// The students × assignments matrix. Pure (no IO) so it's SSR-testable.
#[component]
fn GradeGrid(gradebook: GradebookDto) -> Element {
    let gb = gradebook;
    if gb.students.is_empty() {
        return rsx! {
            EmptyState {
                title: "No enrolled students yet".to_string(),
                description: "Once students enroll, their grades appear here.".to_string(),
            }
        };
    }
    rsx! {
        div { class: "gradebook-grid-wrap",
            table { class: "gradebook-grid",
                thead {
                    tr {
                        th { class: "gradebook-grid__student-col", "Student" }
                        for a in gb.assignments.iter() {
                            th { key: "{a.id}", title: "{a.title}",
                                "{a.title}"
                                if let Some(mp) = a.max_points {
                                    span { class: "gradebook-grid__max", " /{mp}" }
                                }
                            }
                        }
                        th { class: "gradebook-grid__total-col", "Weighted total" }
                    }
                }
                tbody {
                    for st in gb.students.iter() {
                        tr { key: "{st.user_id}",
                            th { scope: "row", class: "gradebook-grid__student-col",
                                "{student_label(st)}"
                            }
                            for a in gb.assignments.iter() {
                                td { key: "{a.id}", class: "gradebook-grid__cell",
                                    "{fmt_grade(cell_grade(st, &a.id))}"
                                }
                            }
                            td { class: "gradebook-grid__total-col", "{fmt_total(st.weighted_total)}" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct CategoryManagerProps {
    api: ApiContext,
    course_id: String,
    categories: Vec<CategoryDto>,
    assignments: Vec<GradebookAssignmentDto>,
    on_changed: EventHandler<()>,
}

#[component]
fn CategoryManager(props: CategoryManagerProps) -> Element {
    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let categories = props.categories.clone();
    let assignments = props.assignments.clone();
    let on_changed = props.on_changed;

    let mut name = use_signal(String::new);
    let mut weight = use_signal(|| "0".to_string());
    let mut submitting = use_signal(|| false);
    let mut toast = use_toast_sender();

    let on_add = {
        let api = api.clone();
        let course_id = course_id.clone();
        move |_| {
            let api = api.clone();
            let course_id = course_id.clone();
            let name_value = name.read().trim().to_string();
            let weight_value: i32 = weight.read().trim().parse().unwrap_or(-1);
            if name_value.is_empty() {
                toast.push(
                    ToastLevel::Danger,
                    "Name required",
                    "Give the category a name.",
                );
                return;
            }
            if !(0..=100).contains(&weight_value) {
                toast.push(
                    ToastLevel::Danger,
                    "Invalid weight",
                    "Weight must be a whole number from 0 to 100.",
                );
                return;
            }
            submitting.set(true);
            spawn(async move {
                match create_category(&api, &course_id, &name_value, weight_value).await {
                    Ok(_) => {
                        toast.push(ToastLevel::Success, "Category added", "");
                        name.set(String::new());
                        weight.set("0".to_string());
                        on_changed.call(());
                    }
                    Err(e) => {
                        toast.push(ToastLevel::Danger, "Couldn't add category", format!("{e}"));
                    }
                }
                submitting.set(false);
            });
        }
    };

    // Category options for the per-assignment Select (with an "Uncategorized"
    // sentinel that maps back to clearing the link).
    let mut select_options = vec![SelectOption {
        value: String::new(),
        label: "Uncategorized".to_string(),
    }];
    for c in &categories {
        select_options.push(SelectOption {
            value: c.id.clone(),
            label: format!("{} ({}%)", c.name, c.weight_percent),
        });
    }

    rsx! {
        Card {
            div { class: "gradebook-categories",
                h3 { "Grade categories" }
                p { class: "muted",
                    "Group assignments into weighted categories. Weights need not sum to 100 — \
                     the total normalizes across the categories a student has grades in."
                }
                if categories.is_empty() {
                    p { class: "muted", "No categories yet. All assignments are weighted equally." }
                } else {
                    ul { class: "gradebook-categories__list",
                        for c in categories.iter() {
                            li { key: "{c.id}", class: "gradebook-categories__item",
                                span { class: "gradebook-categories__name", "{c.name}" }
                                span { class: "gradebook-categories__weight", "{c.weight_percent}%" }
                                Button {
                                    label: "Remove".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    button_type: "button".to_string(),
                                    on_click: {
                                        let api = api.clone();
                                        let course_id = course_id.clone();
                                        let id = c.id.clone();
                                        move |_| {
                                            let api = api.clone();
                                            let course_id = course_id.clone();
                                            let id = id.clone();
                                            spawn(async move {
                                                match delete_category(&api, &course_id, &id).await {
                                                    Ok(_) => {
                                                        toast.push(ToastLevel::Success, "Category removed", "");
                                                        on_changed.call(());
                                                    }
                                                    Err(e) => {
                                                        toast.push(ToastLevel::Danger, "Couldn't remove category", format!("{e}"));
                                                    }
                                                }
                                            });
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                div { class: "gradebook-categories__add",
                    input {
                        class: "ds-input",
                        r#type: "text",
                        placeholder: "Category name",
                        maxlength: "100",
                        value: "{name}",
                        disabled: *submitting.read(),
                        oninput: move |e| name.set(e.value()),
                    }
                    input {
                        class: "ds-input gradebook-categories__weight-input",
                        r#type: "number",
                        min: "0",
                        max: "100",
                        placeholder: "Weight %",
                        value: "{weight}",
                        disabled: *submitting.read(),
                        oninput: move |e| weight.set(e.value()),
                    }
                    Button {
                        label: if *submitting.read() { "Adding…".to_string() } else { "Add category".to_string() },
                        variant: ButtonVariant::Primary,
                        size: ButtonSize::Sm,
                        button_type: "button".to_string(),
                        disabled: *submitting.read(),
                        on_click: on_add,
                    }
                }

                if !assignments.is_empty() {
                    h3 { class: "gradebook-categories__assign-title", "Assignment categories" }
                    ul { class: "gradebook-categories__assignments",
                        for a in assignments.iter() {
                            li { key: "{a.id}", class: "gradebook-categories__assignment-row",
                                span { class: "gradebook-categories__assignment-name", "{a.title}" }
                                Select {
                                    value: a.category_id.clone().unwrap_or_default(),
                                    options: select_options.clone(),
                                    on_change: {
                                        let api = api.clone();
                                        let course_id = course_id.clone();
                                        let assignment_id = a.id.clone();
                                        move |v: String| {
                                            let api = api.clone();
                                            let course_id = course_id.clone();
                                            let assignment_id = assignment_id.clone();
                                            let category_id = if v.is_empty() { None } else { Some(v) };
                                            spawn(async move {
                                                match set_assignment_category(&api, &course_id, &assignment_id, category_id).await {
                                                    Ok(_) => {
                                                        toast.push(ToastLevel::Success, "Category updated", "");
                                                        on_changed.call(());
                                                    }
                                                    Err(e) => {
                                                        toast.push(ToastLevel::Danger, "Couldn't update category", format!("{e}"));
                                                    }
                                                }
                                            });
                                        }
                                    },
                                }
                            }
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

    fn student(name: Option<&str>, email: Option<&str>, total: Option<f64>) -> GradebookStudentDto {
        GradebookStudentDto {
            user_id: "0123456789abcdef".into(),
            display_name: name.map(|s| s.to_string()),
            email: email.map(|s| s.to_string()),
            grades: vec![GradeCellDto {
                assignment_id: "a1".into(),
                numeric_grade: 8.0,
            }],
            weighted_total: total,
        }
    }

    #[test]
    fn student_label_prefers_name_then_email_then_short_id() {
        assert_eq!(
            student_label(&student(
                Some("Ada Lovelace"),
                Some("ada@example.com"),
                None
            )),
            "Ada Lovelace"
        );
        assert_eq!(
            student_label(&student(None, Some("ada@example.com"), None)),
            "ada@example.com"
        );
        assert_eq!(student_label(&student(None, None, None)), "01234567");
    }

    #[test]
    fn fmt_grade_and_total_render_dash_when_absent() {
        assert_eq!(fmt_grade(Some(8.0)), "8.0");
        assert_eq!(fmt_grade(None), "\u{2014}");
        assert_eq!(fmt_total(Some(87.5)), "87.5%");
        assert_eq!(fmt_total(None), "\u{2014}");
    }

    #[test]
    fn grade_grid_renders_students_and_assignment_columns() {
        let gb = GradebookDto {
            course_id: "c".into(),
            categories: vec![],
            assignments: vec![GradebookAssignmentDto {
                id: "a1".into(),
                title: "Essay 1".into(),
                max_points: Some(10),
                category_id: None,
            }],
            students: vec![student(Some("Ada Lovelace"), None, Some(80.0))],
        };
        let mut dom = VirtualDom::new_with_props(GradeGrid, GradeGridProps { gradebook: gb });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("Essay 1"),
            "missing assignment column: {html}"
        );
        assert!(html.contains("Ada Lovelace"), "missing student row: {html}");
        assert!(html.contains("8.0"), "missing grade cell: {html}");
        assert!(html.contains("80.0%"), "missing weighted total: {html}");
        assert!(
            html.contains("Weighted total"),
            "missing total header: {html}"
        );
    }

    #[test]
    fn grade_grid_empty_state_without_students() {
        let gb = GradebookDto {
            course_id: "c".into(),
            categories: vec![],
            assignments: vec![],
            students: vec![],
        };
        let mut dom = VirtualDom::new_with_props(GradeGrid, GradeGridProps { gradebook: gb });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains("No enrolled students yet"),
            "missing empty state: {html}"
        );
    }
}
