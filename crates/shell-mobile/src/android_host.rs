//! JNI boundary for the checked-in Android host. Every inbound string is
//! validated by the renderer-neutral Rust bridge before it reaches app state.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use std::ptr;

fn configured(runtime_name: &str, compiled: Option<&'static str>) -> String {
    let runtime = cfg!(debug_assertions)
        .then(|| std::env::var(runtime_name).ok())
        .flatten()
        .filter(|value| !value.trim().is_empty());
    runtime
        .or_else(|| compiled.map(str::to_owned))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn java_string(env: JNIEnv<'_>, value: String) -> jstring {
    env.new_string(value)
        .map(|value| value.into_raw())
        .unwrap_or(ptr::null_mut())
}

pub(crate) fn validate_config() -> Result<(), String> {
    for (name, value) in [
        (
            "FIREBASE_WEB_API_KEY",
            configured("FIREBASE_WEB_API_KEY", option_env!("FIREBASE_WEB_API_KEY")),
        ),
        (
            "FIREBASE_PROJECT_ID",
            configured("FIREBASE_PROJECT_ID", option_env!("FIREBASE_PROJECT_ID")),
        ),
        (
            "FIREBASE_MESSAGING_SENDER_ID",
            configured(
                "FIREBASE_MESSAGING_SENDER_ID",
                option_env!("FIREBASE_MESSAGING_SENDER_ID"),
            ),
        ),
        (
            "FIREBASE_ANDROID_APP_ID",
            configured(
                "FIREBASE_ANDROID_APP_ID",
                option_env!("FIREBASE_ANDROID_APP_ID"),
            ),
        ),
    ] {
        if value.is_empty() {
            return Err(format!(
                "{name} must be supplied when creating an Android build"
            ));
        }
    }
    Ok(())
}

fn inbound_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    env.get_string(value).ok().map(Into::into)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeFirebaseApiKey(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    java_string(
        env,
        configured("FIREBASE_WEB_API_KEY", option_env!("FIREBASE_WEB_API_KEY")),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeFirebaseProjectId(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    java_string(
        env,
        configured("FIREBASE_PROJECT_ID", option_env!("FIREBASE_PROJECT_ID")),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeFirebaseSenderId(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    java_string(
        env,
        configured(
            "FIREBASE_MESSAGING_SENDER_ID",
            option_env!("FIREBASE_MESSAGING_SENDER_ID"),
        ),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeFirebaseAppId(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    java_string(
        env,
        configured(
            "FIREBASE_ANDROID_APP_ID",
            option_env!("FIREBASE_ANDROID_APP_ID"),
        ),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativePushToken(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    token: JString<'_>,
) -> jboolean {
    let Some(token) = inbound_string(&mut env, &token) else {
        return JNI_FALSE;
    };
    match std::panic::catch_unwind(|| {
        crate::set_push_token_from_host(&token, "android", Some("AulaLite for Android"))
    }) {
        Ok(Ok(())) => JNI_TRUE,
        Ok(Err(_)) | Err(_) => JNI_FALSE,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeDeepLink(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
) -> jboolean {
    let Some(url) = inbound_string(&mut env, &url) else {
        return JNI_FALSE;
    };
    match std::panic::catch_unwind(|| crate::submit_deep_link_from_host(&url)) {
        Ok(Ok(())) => JNI_TRUE,
        Ok(Err(_)) | Err(_) => JNI_FALSE,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_main_AndroidHost_nativeNotificationRoute(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    route: JString<'_>,
) -> jboolean {
    let Some(route) = inbound_string(&mut env, &route) else {
        return JNI_FALSE;
    };
    match std::panic::catch_unwind(|| crate::submit_notification_route_from_host(&route)) {
        Ok(Ok(())) => JNI_TRUE,
        Ok(Err(_)) | Err(_) => JNI_FALSE,
    }
}
