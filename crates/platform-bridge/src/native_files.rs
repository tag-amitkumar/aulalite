//! Native export/save/share primitives.

#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

pub const MAX_EXPORT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved(PathBuf),
    SavedPrivate(PathBuf),
    Cancelled,
    Shared(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveCapability {
    DesktopSaveDialog,
    MobileShareSheet,
    MobilePrivateExportOnly,
    Unsupported,
}

type ShareHandler = dyn Fn(&Path, &str) -> Result<(), String> + Send + Sync + 'static;

fn share_handler() -> &'static RwLock<Option<Arc<ShareHandler>>> {
    static HANDLER: OnceLock<RwLock<Option<Arc<ShareHandler>>>> = OnceLock::new();
    HANDLER.get_or_init(|| RwLock::new(None))
}

/// Install the Android/iOS host share-sheet adapter. Desktop callers normally
/// use a save dialog, while mobile hosts receive the app-private file path.
pub fn install_share_handler(handler: Arc<ShareHandler>) -> Result<(), BridgeError> {
    *share_handler()
        .write()
        .map_err(|_| BridgeError::Io("share handler unavailable".into()))? = Some(handler);
    Ok(())
}

/// Install the checked-in Android Activity share adapter. The adapter exposes
/// only app-private exports through a non-exported, grant-based content
/// provider; paths never become `file://` URLs.
#[cfg(target_os = "android")]
pub fn install_android_share_handler() -> Result<(), BridgeError> {
    install_share_handler(Arc::new(android_share_file))
}

#[cfg(target_os = "android")]
fn android_share_file(path: &Path, mime_type: &str) -> Result<(), String> {
    use jni::objects::{JObject, JValue};

    let context = std::panic::catch_unwind(ndk_context::android_context)
        .map_err(|_| "Android application context is not initialized".to_string())?;
    if context.vm().is_null() || context.context().is_null() {
        return Err("Android application context is unavailable".into());
    }
    // SAFETY: Dioxus/Tao owns these process-lifetime pointers.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| format!("could not access Android JavaVM: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("could not attach to the Android runtime: {error}"))?;
    // SAFETY: ndk-context retains ownership of this global Activity reference.
    let activity = unsafe { JObject::from_raw(context.context().cast::<jni::sys::_jobject>()) };
    let path = env
        .new_string(path.to_string_lossy().as_ref())
        .map_err(|error| format!("could not encode Android export path: {error}"))?;
    let mime_type = env
        .new_string(mime_type)
        .map_err(|error| format!("could not encode Android export type: {error}"))?;
    let path_object = JObject::from(path);
    let mime_object = JObject::from(mime_type);
    let shared = env
        .call_method(
            &activity,
            "shareFileFromRust",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            &[JValue::Object(&path_object), JValue::Object(&mime_object)],
        )
        .and_then(|value| value.z())
        .map_err(|error| format!("could not open the Android share sheet: {error}"))?;
    if shared {
        Ok(())
    } else {
        Err("Android rejected the export share request".into())
    }
}

pub fn save_capability() -> SaveCapability {
    if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
        SaveCapability::DesktopSaveDialog
    } else if cfg!(any(target_os = "android", target_os = "ios")) {
        if share_handler()
            .read()
            .ok()
            .is_some_and(|handler| handler.is_some())
        {
            SaveCapability::MobileShareSheet
        } else {
            SaveCapability::MobilePrivateExportOnly
        }
    } else {
        SaveCapability::Unsupported
    }
}

pub fn safe_filename(suggested: &str) -> String {
    let leaf = Path::new(suggested)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("aulalite-export");
    let cleaned: String = leaf
        .chars()
        .filter(|ch| {
            !ch.is_control() && !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        .take(180)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.');
    if cleaned.is_empty() {
        "aulalite-export".into()
    } else {
        cleaned.into()
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| BridgeError::Io(e.to_string()))?;
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("aulalite-export");
    let temporary = path.with_file_name(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(|e| BridgeError::Io(e.to_string()))?;
    #[cfg(windows)]
    if path.exists() {
        if let Err(error) = std::fs::remove_file(path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(BridgeError::Io(error.to_string()));
        }
    }
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(temporary);
            Err(BridgeError::Io(error.to_string()))
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn private_export_path(filename: &str) -> Result<PathBuf, BridgeError> {
    Ok(crate::native_paths::data_local_dir()?
        .join("Exports")
        .join(filename))
}

pub async fn save_bytes(
    suggested_filename: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<SaveOutcome, BridgeError> {
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err(BridgeError::Io(
            "The export is too large to save safely.".into(),
        ));
    }
    let filename = safe_filename(suggested_filename);

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        let mut dialog = rfd::AsyncFileDialog::new().set_file_name(&filename);
        if let Some((_, extension)) = filename.rsplit_once('.') {
            dialog = dialog.add_filter(mime_type, &[extension]);
        }
        let Some(handle) = dialog.save_file().await else {
            return Ok(SaveOutcome::Cancelled);
        };
        let path = handle.path().to_path_buf();
        write_atomically(&path, bytes)?;
        Ok(SaveOutcome::Saved(path))
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let path = private_export_path(&filename)?;
        write_atomically(&path, bytes)?;
        if let Some(handler) = share_handler()
            .read()
            .map_err(|_| BridgeError::Io("share handler unavailable".into()))?
            .clone()
        {
            handler(&path, mime_type).map_err(BridgeError::Io)?;
            return Ok(SaveOutcome::Shared(path));
        }
        Ok(SaveOutcome::SavedPrivate(path))
    }

    #[cfg(not(any(
        windows,
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        target_os = "ios"
    )))]
    {
        let path = private_export_path(&filename)?;
        write_atomically(&path, bytes)?;
        Ok(SaveOutcome::SavedPrivate(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_is_reduced_to_safe_leaf() {
        assert_eq!(safe_filename("../../report.csv"), "report.csv");
        assert_eq!(safe_filename("bad:<name>?.txt"), "badname.txt");
        assert_eq!(safe_filename("..."), "aulalite-export");
    }
}
