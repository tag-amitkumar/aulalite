//! App-private native filesystem locations.
//!
//! Android application processes do not guarantee a `HOME` environment
//! variable, so `directories::ProjectDirs` can legitimately return `None` on
//! that target. Resolve Android paths from the initialized application context
//! instead; keep the established `ProjectDirs` layout everywhere else.

#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
fn application_project_dirs() -> Result<directories::ProjectDirs, BridgeError> {
    directories::ProjectDirs::from("guru", "Elementors", "AulaLite")
        .ok_or_else(|| BridgeError::Io("could not resolve OS application directory".into()))
}

#[cfg(not(target_os = "android"))]
fn settings_project_dirs() -> Result<directories::ProjectDirs, BridgeError> {
    // Preserve the path used by existing desktop/iOS preferences and debug
    // credentials. Changing these identifiers would silently lose a user's
    // selected workspace after an app update.
    directories::ProjectDirs::from("com", "aulalite", "aulalite")
        .ok_or_else(|| BridgeError::Io("could not resolve OS settings directory".into()))
}

#[cfg(target_os = "android")]
fn android_context_directory(method: &str, label: &str) -> Result<PathBuf, BridgeError> {
    use jni::objects::{JObject, JString};

    let context = std::panic::catch_unwind(ndk_context::android_context)
        .map_err(|_| BridgeError::Io("Android application context is not initialized".into()))?;
    if context.vm().is_null() || context.context().is_null() {
        return Err(BridgeError::Io(
            "Android application context is unavailable".into(),
        ));
    }

    // SAFETY: Tao initializes ndk-context with the process JavaVM and a global
    // Activity reference before the Rust mobile `main` function runs. Both raw
    // pointers were checked above and remain owned by the Android runtime.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| BridgeError::Io(format!("could not access Android JavaVM: {error}")))?;
    let mut env = vm.attach_current_thread().map_err(|error| {
        BridgeError::Io(format!("could not attach to the Android runtime: {error}"))
    })?;
    // SAFETY: ndk-context exposes a valid global `android.content.Context`
    // reference for the lifetime of the activity. `JObject` does not delete the
    // borrowed reference when this wrapper is dropped.
    let context_object =
        unsafe { JObject::from_raw(context.context().cast::<jni::sys::_jobject>()) };

    let directory = env
        .call_method(&context_object, method, "()Ljava/io/File;", &[])
        .map_err(|error| {
            BridgeError::Io(format!(
                "could not resolve Android {label} directory: {error}"
            ))
        })?
        .l()
        .map_err(|error| {
            BridgeError::Io(format!(
                "Android {label} directory had an invalid type: {error}"
            ))
        })?;
    if directory.is_null() {
        return Err(BridgeError::Io(format!(
            "Android {label} directory is unavailable"
        )));
    }

    let absolute_path = env
        .call_method(&directory, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .map_err(|error| BridgeError::Io(format!("could not read Android {label} path: {error}")))?
        .l()
        .map_err(|error| {
            BridgeError::Io(format!("Android {label} path had an invalid type: {error}"))
        })?;
    if absolute_path.is_null() {
        return Err(BridgeError::Io(format!(
            "Android {label} path is unavailable"
        )));
    }
    let absolute_path = JString::from(absolute_path);
    let absolute_path: String = env
        .get_string(&absolute_path)
        .map_err(|error| {
            BridgeError::Io(format!("could not decode Android {label} path: {error}"))
        })?
        .into();
    checked_absolute_path(PathBuf::from(absolute_path), label)
}

fn checked_absolute_path(path: PathBuf, label: &str) -> Result<PathBuf, BridgeError> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(BridgeError::Io(format!(
            "the OS returned an invalid {label} directory"
        )));
    }
    Ok(path)
}

/// Directory for small, non-secret settings and debug-only credential files.
pub fn settings_dir() -> Result<PathBuf, BridgeError> {
    #[cfg(target_os = "android")]
    {
        return Ok(android_context_directory("getFilesDir", "files")?.join("config"));
    }
    #[cfg(not(target_os = "android"))]
    {
        checked_absolute_path(
            settings_project_dirs()?.config_dir().to_path_buf(),
            "settings",
        )
    }
}

/// Directory for bounded, recreatable response data.
pub fn cache_dir() -> Result<PathBuf, BridgeError> {
    #[cfg(target_os = "android")]
    {
        return android_context_directory("getCacheDir", "cache");
    }
    #[cfg(not(target_os = "android"))]
    {
        checked_absolute_path(
            application_project_dirs()?.cache_dir().to_path_buf(),
            "cache",
        )
    }
}

/// Directory for persistent app-private files such as mobile exports.
pub fn data_local_dir() -> Result<PathBuf, BridgeError> {
    #[cfg(target_os = "android")]
    {
        return Ok(android_context_directory("getFilesDir", "files")?.join("data"));
    }
    #[cfg(not(target_os = "android"))]
    {
        checked_absolute_path(
            application_project_dirs()?.data_local_dir().to_path_buf(),
            "application data",
        )
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn host_paths_are_absolute_and_keep_established_layouts() {
        for path in [
            settings_dir().unwrap(),
            cache_dir().unwrap(),
            data_local_dir().unwrap(),
        ] {
            assert!(path.is_absolute(), "{}", path.display());
        }

        let settings = settings_dir().unwrap();
        let cache = cache_dir().unwrap();
        let data = data_local_dir().unwrap();
        assert_ne!(settings, cache);
        assert_ne!(cache, data);
    }

    #[test]
    fn absolute_path_contract_rejects_empty_and_relative_values() {
        assert!(checked_absolute_path(PathBuf::new(), "test").is_err());
        assert!(checked_absolute_path(PathBuf::from("relative/path"), "test").is_err());
    }
}
