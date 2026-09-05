use std::fs;
use std::path::PathBuf;

fn shell_asset(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("public")
        .join("assets")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn design_asset(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("design-system")
        .join("assets")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn shell_asset_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("public")
        .join("assets")
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn design_asset_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("design-system")
        .join("assets")
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

#[test]
fn shell_and_design_system_assets_stay_in_sync() {
    assert_eq!(shell_asset("tokens.css"), design_asset("tokens.css"));
    assert_eq!(
        shell_asset("components.css"),
        design_asset("components.css")
    );
}

#[test]
fn tokens_define_modern_academy_theme_contract() {
    let css = shell_asset("tokens.css");

    for expected in [
        "--color-paper",
        "--color-ink",
        "--color-accent",
        "--color-accent-strong",
        "--color-live",
        "--font-display",
        "--font-body",
        "--radius-md: 8px",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }

    for expected in [
        "--color-paper",
        "--color-ink",
        "--color-accent",
        "--color-gold",
        "--color-oxblood",
        "--motion-fast",
        "--motion-page",
        "--font-display",
        "--font-body",
        "--radius-md: 8px",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }

    for expected in [
        "--ease-snappy",
        "--ease-decelerate",
        "--ease-accelerate",
        "--ease-spring",
        "--ease-page-in",
        "--duration-fast",
        "--duration-medium",
        "--duration-slow",
        "--duration-page-in",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }

    for expected in [
        // Type scale
        "--text-xs",
        "--text-sm",
        "--text-base",
        "--text-md",
        "--text-lg",
        "--text-xl",
        "--text-2xl",
        "--text-3xl",
        "--text-4xl",
        "--text-5xl",
        "--text-6xl",
        "--text-7xl",
        // Leading scale
        "--leading-xs",
        "--leading-sm",
        "--leading-base",
        "--leading-md",
        "--leading-lg",
        "--leading-xl",
        "--leading-2xl",
        "--leading-3xl",
        "--leading-4xl",
        "--leading-5xl",
        "--leading-6xl",
        "--leading-7xl",
        // Spacing extensions
        "--space-0",
        "--space-10",
        "--space-12",
        // Radius scale
        "--radius-xs",
        "--radius-lg",
        "--radius-xl",
        "--radius-2xl",
        "--radius-full",
        // Shadow elevation scale
        "--shadow-xs",
        "--shadow-xl",
        "--shadow-2xl",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }

    for expected in [
        "--color-ink-sidebar",
        "--color-ink-stage",
        "--color-ink-stage-deep",
        "--color-on-ink-strong",
        "--color-on-ink-subtle",
        "--color-banner-warm",
        "--color-banner-warm-border",
        "--color-state-error-bg",
        "--color-state-error-border",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }

    // Font-family tokens lead with the self-hosted faces.
    assert!(
        css.contains("\"Source Serif 4\""),
        "tokens.css must reference Source Serif 4 in --font-display"
    );
    assert!(
        css.contains("\"Inter\""),
        "tokens.css must reference Inter in --font-body"
    );

    // Dark theme keyed on the kinetics ThemeProvider attribute, with token
    // overrides only (semantic aliases flip; chromatic ramps stay invariant).
    assert!(
        css.contains("[data-ui-theme=\"dark\"]"),
        "tokens.css must define the [data-ui-theme=\"dark\"] theme block"
    );
    assert!(
        css.contains("color-scheme: dark"),
        "dark theme must opt native widgets into dark rendering"
    );
}

#[test]
fn brand_chromatic_ramps_are_complete() {
    let css = shell_asset("tokens.css");
    for color in ["green", "gold", "navy", "oxblood"] {
        for step in [
            "50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950",
        ] {
            let expected = format!("--color-{color}-{step}");
            assert!(
                css.contains(&expected),
                "tokens.css missing expected ramp step `{expected}`"
            );
        }
    }
}

#[test]
fn semantic_and_neutral_ramps_are_complete() {
    let css = shell_asset("tokens.css");
    for color in ["info", "success", "warning", "danger", "neutral"] {
        for step in [
            "50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950",
        ] {
            let expected = format!("--color-{color}-{step}");
            assert!(
                css.contains(&expected),
                "tokens.css missing expected ramp step `{expected}`"
            );
        }
    }
}

#[test]
fn components_cover_editorial_web_surface() {
    let css = shell_asset("components.css");

    for selector in [
        ".auth-composite",
        ".auth-local-panel",
        ".auth-title",
        ".app-shell-layout",
        ".app-side",
        ".app-topbar",
        ".page-header",
        ".dashboard-grid",
        ".dashboard-stat",
        ".course-cards",
        ".course-card-cover-empty",
        ".course-detail-header",
        ".schedule-list",
        ".live-room-view",
        ".live-room-broadcast",
        ".live-room-sidebars",
    ] {
        assert!(
            css.contains(selector),
            "components.css missing expected selector `{selector}`"
        );
    }

    for selector in [
        ".aula-logo",
        ".ui-icon",
        ".auth-hero-visual",
        ".system-state",
        ".skeleton-line",
        ".motion-page",
        ".dashboard-hero",
        ".course-card-art",
        ".assignment-shell",
        ".live-room-stage",
    ] {
        assert!(
            css.contains(selector),
            "components.css missing expected selector `{selector}`"
        );
    }
}

#[test]
fn keyboard_focus_treatments_are_opaque_and_high_contrast() {
    let tokens = shell_asset("tokens.css");
    for line in tokens
        .lines()
        .filter(|line| line.trim_start().starts_with("--color-focus:"))
    {
        assert!(
            !line.contains("rgba(") && !line.contains("transparent"),
            "focus tokens must not lose contrast through alpha: {line}"
        );
    }

    let components = shell_asset("components.css");
    assert!(
        components.contains("outline: 3px solid #765019;"),
        "shared public pages must retain an opaque focus color"
    );
    let cinematic_focus = components
        .rfind("outline: 3px solid #f3ffcf;")
        .expect("cinematic marketing focus ring missing");
    let legacy_focus = components
        .rfind("outline: 3px solid #765019;")
        .expect("shared public focus ring missing");
    assert!(
        cinematic_focus > legacy_focus,
        "the two-tone cinematic focus treatment must override the legacy marketing rule"
    );
    assert!(
        components.contains("box-shadow: 0 0 0 7px #13251f;"),
        "cinematic focus treatment needs a dark outer ring on light surfaces"
    );
    assert!(
        !components.contains("outline: 3px solid rgba(188, 139, 57, 0.55)"),
        "low-contrast translucent marketing focus outline returned"
    );
}

#[test]
fn cinematic_marketing_assets_are_self_hosted_and_motion_safe() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let html = fs::read_to_string(root.join("index.html")).expect("read index.html");
    let motion = shell_asset("marketing-motion.js");
    let css = shell_asset("components.css");
    let landing = fs::read_to_string(root.join("src/routes/landing.rs")).expect("read landing");

    assert!(html.contains("/assets/marketing-motion.js"));
    assert!(!html.contains("cdn.jsdelivr.net/npm/gsap"));
    for expected in [
        "/assets/vendor/gsap.min.js",
        "/assets/vendor/ScrollTrigger.min.js",
        "prefers-reduced-motion: reduce",
        "context.revert()",
    ] {
        assert!(
            motion.contains(expected),
            "marketing motion bridge missing `{expected}`"
        );
    }

    for asset in [
        "assets/vendor/gsap.min.js",
        "assets/vendor/ScrollTrigger.min.js",
    ] {
        let metadata = fs::metadata(root.join("public").join(asset))
            .unwrap_or_else(|err| panic!("missing self-hosted {asset}: {err}"));
        assert!(metadata.len() > 30_000, "vendor asset too small: {asset}");
    }

    for contract in [
        "font-family: \"Outfit\"",
        "grid-template-columns: repeat(12, minmax(0, 1fr))",
        "grid-auto-flow: dense",
        ".marketing-stack-card",
        "@media (prefers-reduced-motion: reduce)",
    ] {
        assert!(css.contains(contract), "cinematic CSS missing `{contract}`");
    }

    assert!(landing.contains("max-w-6xl"));
    assert!(!landing.contains("marketing-feature__number"));
}

#[test]
fn brand_assets_exist_and_stay_in_sync() {
    for asset in [
        "brand/aulalite-mark.svg",
        "brand/aulalite-wordmark.svg",
        "brand/academy-hero.png",
        "brand/course-cover-academy.png",
    ] {
        let shell = shell_asset_bytes(asset);
        let design = design_asset_bytes(asset);
        assert!(shell.len() > 100, "asset too small: {asset}");
        assert_eq!(shell, design, "asset not mirrored: {asset}");
    }
}

#[test]
fn fonts_exist_and_stay_in_sync() {
    for asset in [
        "fonts/SourceSerif4-Variable.woff2",
        "fonts/SourceSerif4-Italic-Variable.woff2",
        "fonts/Inter-Variable.woff2",
        "fonts/Inter-Italic-Variable.woff2",
        "fonts/outfit-latin-wght-normal.woff2",
        "fonts/LICENSE.md",
    ] {
        let shell = shell_asset_bytes(asset);
        let design = design_asset_bytes(asset);
        assert!(shell.len() > 100, "font asset too small: {asset}");
        assert_eq!(shell, design, "font asset not mirrored: {asset}");
    }
}

#[test]
fn index_references_brand_metadata() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("index.html");
    let html = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    assert!(html.contains("AulaLite"));
    assert!(html.contains("/assets/brand/aulalite-mark.svg"));
    assert!(html.contains("/assets/brand/app-icon-32.png"));
    assert!(html.contains("/assets/brand/app-icon-256.png"));
    assert!(html.contains("theme-color"));
    assert!(html.contains("property=\"og:title\""));
    assert!(html.contains("name=\"twitter:card\""));
    assert!(html.contains("name=\"robots\""));
    assert!(html.contains("viewport-fit=cover"));
    assert!(html.contains("name=\"mobile-web-app-capable\""));
    assert!(html.contains("name=\"apple-mobile-web-app-capable\""));
    assert!(html.contains("<noscript>"));
}

#[test]
fn runtime_firebase_config_is_shared_and_loaded_before_bridges() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let html = fs::read_to_string(root.join("index.html")).expect("read index.html");
    let runtime_position = html
        .find("/runtime-config.js?v=__AULALITE_RUNTIME_CONFIG_VERSION__")
        .expect("index must load versioned runtime config");
    let auth_bridge_position = html
        .find("/assets/firebase-bridge.js")
        .expect("index must load Firebase auth bridge");
    assert!(
        runtime_position < auth_bridge_position,
        "runtime config must execute before Firebase bridges"
    );

    let local_config = fs::read_to_string(root.join("public/runtime-config.js"))
        .expect("read local runtime config placeholder");
    assert!(local_config.contains("__AULALITE_FIREBASE_API_KEY__ = \"\""));
    assert!(local_config.contains("__AULALITE_FCM_VAPID_KEY__ = \"\""));
    assert!(local_config.contains("__AULALITE_API_BASE_URL__ = \"\""));
    assert!(local_config.contains("__AULALITE_APP_ORIGIN__ = \"\""));
    assert!(local_config.contains("__AULALITE_ADMIN_ORIGIN__ = \"\""));

    let messaging_worker = fs::read_to_string(root.join("public/firebase-messaging-sw.js"))
        .expect("read Firebase messaging worker");
    assert!(messaging_worker.contains("importScripts(\"/runtime-config.js\")"));
    assert!(messaging_worker.contains("self.__AULALITE_FIREBASE_API_KEY__"));
    assert!(messaging_worker.contains("if (firebaseConfigured)"));
    assert!(messaging_worker.contains("notificationclick"));
    assert!(messaging_worker.contains("url.origin !== self.location.origin"));

    let entrypoint =
        fs::read_to_string(root.join("docker-entrypoint.d/40-aulalite-runtime-config.sh"))
            .expect("read runtime config entrypoint");
    assert!(entrypoint.contains("FIREBASE_WEB_API_KEY"));
    assert!(entrypoint.contains("AULALITE_API_BASE_URL"));
    assert!(entrypoint.contains("AULALITE_APP_ORIGIN"));
    assert!(entrypoint.contains("AULALITE_ADMIN_ORIGIN"));
    assert!(entrypoint.contains("global.location.pathname === \"/\""));
    assert!(entrypoint.contains("AULALITE_REQUIRE_FIREBASE_CONFIG=false"));
    assert!(entrypoint.contains("base64_value"));
    assert!(entrypoint.contains("find \"$app_root\" -type f"));
    assert!(entrypoint.contains("! -name 'runtime-config.js'"));
}

#[test]
fn specialist_media_code_is_loaded_only_on_demand() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let html = fs::read_to_string(root.join("index.html")).expect("read index.html");
    assert!(html.contains("/assets/feature-loader.js"));
    assert!(
        !html.contains("import Hls from \"/vendor/hls.js\""),
        "the 413 KB HLS player must not be part of every page load"
    );

    let loader = fs::read_to_string(root.join("public/assets/feature-loader.js"))
        .expect("read feature loader");
    assert!(loader.contains("import(\"/vendor/hls.js\")"));
    assert!(loader.contains("video.canPlayType"));
    assert!(loader.contains("hls.destroy()"));
    assert!(loader.contains("hlsModulePromise"));
}

#[test]
fn pwa_cache_is_deployment_scoped_bounded_and_privacy_safe() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let html = fs::read_to_string(root.join("index.html")).expect("read index.html");
    assert!(html.contains("/assets/app-bootstrap.js?v=__AULALITE_RUNTIME_CONFIG_VERSION__"));
    assert!(!html.contains("<script>"), "inline scripts weaken the CSP");

    let bootstrap = fs::read_to_string(root.join("public/assets/app-bootstrap.js"))
        .expect("read app bootstrap");
    assert!(bootstrap.contains("/service-worker.js?v="));
    assert!(bootstrap.contains("document.currentScript.src"));

    let worker =
        fs::read_to_string(root.join("public/service-worker.js")).expect("read service worker");
    for contract in [
        "DEPLOY_VERSION",
        "MAX_RUNTIME_ENTRIES",
        "trimRuntimeCache",
        "event.waitUntil(refresh.catch",
        "request.headers.has(\"authorization\")",
        "\"/runtime-config.js\"",
        "url.pathname.startsWith(\"/v1/\")",
        "key.includes(\"signature\")",
        "<html lang=\"en\">",
        "You’re offline",
    ] {
        assert!(
            worker.contains(contract),
            "service worker missing `{contract}`"
        );
    }

    let fcm =
        fs::read_to_string(root.join("public/assets/fcm-bridge.js")).expect("read FCM bridge");
    assert!(fcm.contains("/firebase-cloud-messaging-push-scope"));

    let manifest =
        fs::read_to_string(root.join("public/manifest.webmanifest")).expect("read web manifest");
    assert!(manifest.contains("/assets/brand/app-icon-256.png"));
    assert!(manifest.contains("/assets/brand/app-icon-512.png"));
    for asset in [
        "assets/brand/app-icon-32.png",
        "assets/brand/app-icon-256.png",
        "assets/brand/app-icon-512.png",
    ] {
        assert!(root.join("public").join(asset).is_file(), "missing {asset}");
    }
}

#[test]
fn production_static_delivery_compresses_and_revalidates_workers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dockerfile = fs::read_to_string(root.join("Dockerfile")).expect("read Dockerfile");
    assert!(dockerfile.contains("gzip on;"));
    assert!(dockerfile.contains("application/wasm"));
    assert!(dockerfile.contains("rm -rf /app/target/dx/shell-web/release/web"));
    assert!(dockerfile.contains("location = /service-worker.js"));
    assert!(dockerfile.contains("Cache-Control \"no-cache, must-revalidate\""));
    assert!(dockerfile.contains("Content-Security-Policy"));
    assert!(!dockerfile.contains("script-src 'self' 'unsafe-inline'"));
    assert!(dockerfile.contains("location = /lti/landing"));
    assert!(dockerfile.contains("rewrite ^ /index.html break;"));
    assert!(dockerfile.contains("stale-while-revalidate=604800"));
}
