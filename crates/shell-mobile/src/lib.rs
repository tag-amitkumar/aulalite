//! Library target for mobile linking (iOS/Android produce staticlib/cdylib).
//!
//! Mobile intentionally mounts the same route graph and provider stack as the
//! web/desktop shells. A previous reduced route enum omitted destinations that
//! the shared navigation emitted (`/signup`, `/forgot`, `/courses`, quizzes,
//! settings, and administration), leaving ordinary taps unresolved.

mod route_enum;

#[cfg(target_os = "android")]
mod android_host;

use dioxus::prelude::*;

pub use route_enum::MobileRoute;

/// Android/iOS lifecycle glue forwards universal links and custom-scheme URLs
/// here. The shared route layout wakes up and completes the callback.
pub fn submit_deep_link_from_host(url: &str) -> Result<(), String> {
    platform_bridge::deep_links::submit_deep_link_from_host(url).map_err(|error| error.to_string())
}

/// Android FCM / iOS APNs-through-FCM lifecycle hook. Rotation is retained in
/// secure storage until the notification settings sync reaches the backend.
pub fn set_push_token_from_host(
    token: &str,
    platform: &str,
    label: Option<&str>,
) -> Result<(), String> {
    platform_bridge::native_push::set_registration_token_from_host(token, platform, label)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Native notification taps are kept separate from authentication callbacks
/// so their routes can be consumed exactly once by the shared router.
pub fn submit_notification_route_from_host(route: &str) -> Result<(), String> {
    platform_bridge::deep_links::submit_notification_route_from_host(route)
        .map_err(|error| error.to_string())
}

#[component]
pub fn MobileApp() -> Element {
    if let Err(error) = platform_bridge::native::validate_runtime_config() {
        return native_configuration_error(error.to_string());
    }
    #[cfg(target_os = "android")]
    if let Err(error) = android_host::validate_config() {
        return native_configuration_error(error);
    }

    rsx! { shell_web::App {} }
}

fn native_configuration_error(detail: String) -> Element {
    rsx! {
        main {
            class: "native-config-screen",
            style: "min-height:100vh;display:grid;place-items:center;padding:28px;background:linear-gradient(145deg,#0d172a,#10251f);color:#f7efe0;font-family:system-ui,sans-serif",
            section {
                style: "width:min(100%,520px);padding:28px;border:1px solid rgba(231,207,153,.3);border-radius:24px;background:rgba(255,255,255,.08);box-shadow:0 24px 70px rgba(0,0,0,.32)",
                p { style: "margin:0 0 8px;color:#e7cf99;font-weight:700;letter-spacing:.08em;text-transform:uppercase", "AulaLite setup" }
                h1 { style: "margin:0 0 12px;font-size:clamp(1.6rem,7vw,2.5rem)", "This app needs its secure connection configured" }
                p { style: "margin:0 0 18px;line-height:1.6;color:rgba(247,239,224,.82)",
                    "The installed build is missing its production API settings. Contact your school or AulaLite support and include the detail below."
                }
                code { style: "display:block;padding:14px;border-radius:12px;background:rgba(0,0,0,.25);overflow-wrap:anywhere;color:#f7efe0", "{detail}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_app_is_exported_from_library_target() {
        let app: fn() -> Element = MobileApp;
        let _ = app;
    }

    #[test]
    fn mobile_app_renders_auth_splash_before_bootstrap_ready() {
        let mut dom = VirtualDom::new(MobileApp);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("app-auth-splash"), "html: {html}");
        assert!(html.contains("Aula"), "html: {html}");
    }

    #[test]
    fn mobile_route_graph_resolves_every_shared_navigation_family() {
        use std::str::FromStr;

        for path in [
            "/login",
            "/signup",
            "/forgot",
            "/courses",
            "/courses/physics/assignments/a1",
            "/courses/physics/quizzes/q1",
            "/schedule",
            "/calendar",
            "/settings/notifications",
            "/admin",
            "/platform",
        ] {
            MobileRoute::from_str(path)
                .unwrap_or_else(|error| panic!("mobile route {path} did not resolve: {error}"));
        }
    }
}
