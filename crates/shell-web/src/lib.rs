// crates/shell-web/src/lib.rs
//! Web shell crate. The same `App` is launched by `main.rs` (wasm) and
//! re-exported for `shell-desktop`.
// Native shells compile the complete web route graph so deep links and role
// navigation stay in parity; a few browser-only helpers are consequently dead
// on those targets. Keep web lint coverage strict.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub mod auth_bootstrap;
pub mod contexts;
pub mod native_navigation;
pub mod route_enum;
pub mod routes;

use design_system::ToastProvider;
use dioxus::prelude::*;
use dioxus_router::Router;

#[component]
pub fn App() -> Element {
    let auth = auth_bootstrap::use_auth_bootstrap();

    rsx! {
        design_system::NativeBaseStyles {}
        design_system::KineticsStyles {}
        // Reactive theme/density context for kinetics components. The provider
        // watches the `data-ui-theme` / `data-ui-density` attributes that the
        // boot script and `design_system::apply_theme_preference` set on
        // <html>, so `use_theme_mode()` consumers stay in sync with the CSS.
        design_system::LocaleProvider {
            design_system::kinetics_ui::ThemeProvider {
                ToastProvider {
                    // Hold routing until the auth bootstrap resolves: every route
                    // guard treats "no user" as logged-out, so rendering them
                    // mid-bootstrap bounced signed-in users to /login on reload.
                    if *auth.auth_ready.read() {
                        Router::<route_enum::Route> {}
                    } else {
                        auth_bootstrap::AuthSplash {}
                    }
                }
            }
        }
    }
}
