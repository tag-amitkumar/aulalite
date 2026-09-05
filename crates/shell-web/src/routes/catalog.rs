// crates/shell-web/src/routes/catalog.rs
//
// `/catalog` — browse published courses in the workspace that are open for
// self-enrollment and join them directly (no code or invite). Seat caps are
// enforced server-side; the same upgrade prompt surfaces on conflict.
use design_system::PageHeader;
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::catalog_view::CatalogView;

use crate::route_enum::Route;
use crate::routes::use_user_context;

#[component]
pub fn CatalogPage() -> Element {
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
            div { class: "catalog-page",
                PageHeader {
                    title: "Course catalog".to_string(),
                    kicker: "Discover".to_string(),
                    subtitle: "Open courses in this workspace — join any of them instantly, no code needed.".to_string(),
                }
                CatalogView {
                    on_joined: move |slug: String| {
                        if slug.is_empty() {
                            // Joined but slug lookup failed — refresh via dashboard.
                            nav.push(Route::Dashboard {});
                        } else {
                            nav.push(Route::CourseDetail { slug });
                        }
                    },
                }
            }
        }
    }
}
