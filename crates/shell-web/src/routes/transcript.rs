// crates/shell-web/src/routes/transcript.rs
//
// `/transcript` — the signed-in learner's consolidated academic record:
// per-course weighted total + letter grade, lesson progress and certificates,
// printable to PDF from the browser.
use design_system::PageHeader;
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::transcript_view::TranscriptView;

use crate::route_enum::Route;
use crate::routes::use_user_context;

#[component]
pub fn TranscriptPage() -> Element {
    let nav = use_navigator();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };
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
            div { class: "transcript-page",
                PageHeader {
                    title: "My transcript".to_string(),
                    kicker: "Academic record".to_string(),
                    subtitle: "Your courses in this workspace with grades, progress, and issued certificates.".to_string(),
                    variant: design_system::PageHeaderVariant::Hero,
                }
                TranscriptView {}
            }
        }
    }
}
