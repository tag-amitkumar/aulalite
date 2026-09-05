// crates/shell-web/src/routes/my_schedule.rs
use design_system::PageHeader;
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::schedule_view::{ScheduleEntry, ScheduleView};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn MySchedule() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let schedule = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_schedule(&api).await }
        }
    });
    let courses = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_courses(&api).await }
        }
    });

    // Build the slug lookup map from courses.
    let courses_snap = courses.read_unchecked();
    let course_slug_by_id: std::collections::HashMap<String, String> = match courses_snap.as_ref() {
        Some(Ok(cs)) => cs
            .iter()
            .map(|c| (c.course_id.clone(), c.slug.clone()))
            .collect(),
        _ => std::collections::HashMap::new(),
    };
    drop(courses_snap);

    let schedule_snap = schedule.read_unchecked();
    let entries: Vec<ScheduleEntry> = match schedule_snap.as_ref() {
        Some(Ok(rows)) => rows
            .iter()
            .map(|s| ScheduleEntry {
                session_id: s.session_id.clone(),
                course_title: s.course_title.clone(),
                course_slug: course_slug_by_id
                    .get(&s.course_id)
                    .cloned()
                    .unwrap_or_default(),
                title: s.title.clone(),
                starts_at_display: s.starts_at.clone(),
                duration_minutes: s.duration_minutes,
                status: s.status.clone(),
                diverged: false,
                can_edit: user.can_teach(),
            })
            .collect(),
        _ => vec![],
    };
    drop(schedule_snap);

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            div { class: "my-schedule-page motion-page",
                PageHeader {
                    title: "Schedule".to_string(),
                    kicker: "Live sessions".to_string(),
                }
                ScheduleView {
                    entries: entries,
                    on_cancel: move |_id: String| {},
                    on_reschedule: move |_id: String| {},
                }
            }
        }
    }
}
