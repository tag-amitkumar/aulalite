fn main() {
    // Local runs may use the ignored repository .env. Store builds embed only
    // explicitly supplied publishable API/Firebase configuration.
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();
    platform_bridge::deep_links::initialize_from_process_args(std::env::args());
    #[cfg(target_os = "android")]
    if let Err(error) = platform_bridge::native_files::install_android_share_handler() {
        eprintln!("AulaLite could not install Android sharing: {error}");
    }
    let config = dioxus::mobile::Config::new().with_custom_event_handler(|event, _target| {
        if let dioxus::mobile::tao::event::Event::Opened { urls } = event {
            for url in urls {
                let _ = platform_bridge::deep_links::submit_deep_link_from_host(url.as_str());
            }
        }
    });
    dioxus::LaunchBuilder::mobile()
        .with_cfg(config)
        .launch(shell_mobile::MobileApp);
}
