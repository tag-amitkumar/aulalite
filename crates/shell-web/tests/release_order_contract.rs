use std::{fs, path::PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("shell-web must be inside crates/")
        .to_path_buf()
}

fn release_workflow() -> String {
    fs::read_to_string(workspace_root().join(".github/workflows/release-candidate.yml"))
        .expect("ordered release workflow")
        .replace("\r\n", "\n")
}

#[test]
fn dioxus_framework_and_release_cli_stay_in_lockstep() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("workspace Cargo.toml");
    let workflow = release_workflow();

    assert!(cargo.contains("dioxus = \"=0.7.9\""));
    assert!(cargo.contains("dioxus-router = \"=0.7.9\""));
    assert!(workflow.contains("DX_VERSION: \"0.7.9\""));
}

#[test]
fn release_candidate_platforms_form_a_strict_dependency_chain() {
    let workflow = release_workflow();

    let web = workflow.find("\n  web:\n").expect("web release job");
    let android = workflow
        .find("\n  android:\n")
        .expect("Android release job");
    let windows = workflow
        .find("\n  windows:\n")
        .expect("Windows release job");
    let ios = workflow.find("\n  ios:\n").expect("iOS release job");

    assert!(web < android && android < windows && windows < ios);
    assert!(workflow[web..android].contains("needs: quality"));
    assert!(workflow[android..windows].contains("needs: web"));
    assert!(workflow[windows..ios].contains("needs: android"));
    assert!(workflow[ios..].contains("needs: windows"));
}

#[test]
fn unsigned_candidates_do_not_embed_distribution_credentials() {
    let workflow = release_workflow();

    assert!(workflow.contains("unsigned release candidates"));
    for forbidden in [
        "ANDROID_KEYSTORE_PASSWORD",
        "APPLE_DISTRIBUTION_CERTIFICATE",
        "WINDOWS_SIGNING_CERTIFICATE",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "distribution secret {forbidden} belongs in a separate approved signing workflow"
        );
    }
}

#[test]
fn native_candidates_keep_store_permissions_and_icons_platform_specific() {
    let root = workspace_root();
    let workflow = release_workflow();
    let mobile = fs::read_to_string(root.join("crates/shell-mobile/Dioxus.toml"))
        .expect("mobile Dioxus configuration");

    assert!(workflow.contains("--platform windows --release --features bundle"));
    assert!(workflow.contains("platforms;android-36"));
    assert!(workflow.contains("--target aarch64-linux-android --release --package-types aab"));
    assert!(mobile.contains("target_sdk = 36"));
    assert!(mobile.contains("compile_sdk = 36"));
    assert!(mobile.contains("app-icon-ios-1024.png"));
    assert!(!mobile.contains("photos ="));
    assert!(!mobile.contains("media-library ="));
}

#[test]
fn android_host_contract_is_checked_in_and_release_configured() {
    let root = workspace_root();
    let workflow = release_workflow();
    let mobile = fs::read_to_string(root.join("crates/shell-mobile/Dioxus.toml"))
        .expect("mobile Dioxus configuration");
    let manifest = fs::read_to_string(root.join("crates/shell-mobile/android/AndroidManifest.xml"))
        .expect("Android manifest");
    let activity = fs::read_to_string(root.join("crates/shell-mobile/android/MainActivity.kt"))
        .expect("Android activity");

    assert!(mobile.contains("android_main_activity = \"android/MainActivity.kt\""));
    assert!(mobile.contains("android_manifest = \"android/AndroidManifest.xml\""));
    assert!(mobile.contains("com.google.firebase:firebase-messaging:25.1.0"));
    for required in [
        "android.permission.POST_NOTIFICATIONS",
        "AulaLiteMessagingService",
        "AulaLiteExportProvider",
        "android:usesCleartextTraffic=\"false\"",
        "android:autoVerify=\"true\"",
    ] {
        assert!(
            manifest.contains(required),
            "manifest is missing {required}"
        );
    }
    assert!(!manifest.contains("networkSecurityConfig"));
    for required in [
        "override fun onNewIntent",
        "override fun onNewToken",
        "requestPushPermissionFromRust",
        "deletePushTokenFromRust",
        "shareFileFromRust",
        "nativeNotificationRoute",
    ] {
        assert!(
            activity.contains(required),
            "Android host is missing {required}"
        );
    }
    for required in [
        "FIREBASE_PROJECT_ID",
        "FIREBASE_MESSAGING_SENDER_ID",
        "FIREBASE_ANDROID_APP_ID",
    ] {
        assert!(
            workflow.contains(required),
            "release workflow is missing {required}"
        );
    }
}
