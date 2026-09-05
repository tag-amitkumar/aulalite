// crates/shell-web/src/routes/assignments_list.rs
//
// Course assignments route. Thin delegate: the list renders INSIDE the course
// shell (header + tab bar) via `course_detail::render_with_tab`, like every
// other course tab — this page previously dropped the chrome entirely.
use dioxus::prelude::*;

#[component]
pub fn AssignmentList(slug: String) -> Element {
    crate::routes::course_detail::render_with_tab(slug, "assignments")
}
