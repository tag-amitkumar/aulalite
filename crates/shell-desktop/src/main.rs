// crates/shell-desktop/src/main.rs
#![cfg_attr(all(windows, feature = "bundle"), windows_subsystem = "windows")]

use dioxus::prelude::*;
use std::sync::OnceLock;

static STARTUP_ERROR: OnceLock<String> = OnceLock::new();

fn main() {
    // Local development may use the ignored workspace .env. Release builds
    // embed only explicitly supplied publishable API/Firebase configuration.
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();
    platform_bridge::deep_links::initialize_from_process_args(std::env::args());
    if let Err(error) = platform_bridge::native::validate_runtime_config() {
        let _ = STARTUP_ERROR.set(error.to_string());
        launch_desktop(NativeConfigurationError);
        return;
    }
    launch_desktop(shell_web::App);
}

fn launch_desktop(app: fn() -> Element) {
    let config = dioxus::desktop::Config::new().with_custom_event_handler(|event, _target| {
        if let dioxus::desktop::tao::event::Event::Opened { urls } = event {
            for url in urls {
                let _ = platform_bridge::deep_links::submit_deep_link_from_host(url.as_str());
            }
        }
    });
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(app);
}

#[component]
#[allow(non_snake_case)]
fn NativeConfigurationError() -> Element {
    let detail = STARTUP_ERROR
        .get()
        .cloned()
        .unwrap_or_else(|| "Native application configuration is incomplete.".into());
    rsx! {
        main {
            class: "native-config-screen",
            style: "min-height:100vh;display:grid;place-items:center;padding:32px;background:linear-gradient(145deg,#0d172a,#10251f);color:#f7efe0;font-family:system-ui,sans-serif",
            section {
                style: "width:min(100%,560px);padding:32px;border:1px solid rgba(231,207,153,.3);border-radius:24px;background:rgba(255,255,255,.08);box-shadow:0 24px 70px rgba(0,0,0,.32)",
                p { style: "margin:0 0 8px;color:#e7cf99;font-weight:700;letter-spacing:.08em;text-transform:uppercase", "AulaLite setup" }
                h1 { style: "margin:0 0 12px;font-size:clamp(1.7rem,5vw,2.6rem)", "This app needs its secure connection configured" }
                p { style: "margin:0 0 18px;line-height:1.6;color:rgba(247,239,224,.82)",
                    "The installed build is missing its production API settings. Contact your organization administrator or AulaLite support and include the detail below."
                }
                code { style: "display:block;padding:14px;border-radius:12px;background:rgba(0,0,0,.25);overflow-wrap:anywhere;color:#f7efe0", "{detail}" }
            }
        }
    }
}
