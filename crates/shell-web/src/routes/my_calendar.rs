// crates/shell-web/src/routes/my_calendar.rs
//
// Top-level personal calendar (/calendar). Available to every signed-in role:
// lists the caller's own visible live sessions + assignment due dates from
// GET /v1/me/calendar, grouped by day, each linking out. The CalendarView
// component (in features-courses) owns the agenda rendering AND the
// authenticated ".ics" Download / Subscribe affordances, so this route just
// fetches the events and hands them over inside the AppShell.
use design_system::{PageHeader, SkeletonCard};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::calendar_view::{list_my_calendar, CalendarEventDto, CalendarView};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn MyCalendar() -> Element {
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

    let events = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { list_my_calendar(&api, None, None).await }
        }
    });

    let snap = events.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) => {
            let rows: Vec<CalendarEventDto> = rows.clone();
            rsx! { CalendarView { events: rows } }
        }
        Some(Err(e)) => rsx! {
            p { class: "error", "Could not load your calendar: {e}" }
        },
        None => rsx! { SkeletonCard { height: "240px".to_string() } },
    };
    drop(snap);

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
            div { class: "my-calendar-page motion-page",
                PageHeader {
                    title: "Calendar".to_string(),
                    kicker: "Sessions & due dates".to_string(),
                }
                { body }
            }
        }
    }
}
