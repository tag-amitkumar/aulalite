#![cfg(not(target_arch = "wasm32"))]

//! Small, non-secret native preferences persisted under the OS application
//! config directory. Authentication material belongs in `native_store`; this
//! file holds only the last selected workspace UUID.

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_PREFERENCES_BYTES: u64 = 16 * 1024;
static PREFERENCES_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct NativePreferences {
    selected_workspace_id: Option<String>,
}

fn preferences_lock() -> &'static Mutex<()> {
    PREFERENCES_LOCK.get_or_init(|| Mutex::new(()))
}

fn preferences_path() -> Result<PathBuf, BridgeError> {
    Ok(crate::native_paths::settings_dir()?.join("preferences.json"))
}

fn normalized_workspace_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > 64 {
        return None;
    }
    uuid::Uuid::parse_str(value)
        .ok()
        .map(|id| id.hyphenated().to_string())
}

fn read_preferences(path: &Path) -> Result<NativePreferences, BridgeError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NativePreferences::default())
        }
        Err(error) => return Err(BridgeError::Io(error.to_string())),
    };
    if metadata.len() > MAX_PREFERENCES_BYTES {
        return Err(BridgeError::Io(
            "native preferences file is too large".into(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| BridgeError::Io(error.to_string()))?;
    let mut preferences: NativePreferences = serde_json::from_slice(&bytes)
        .map_err(|_| BridgeError::Io("native preferences file is invalid".into()))?;
    preferences.selected_workspace_id = preferences
        .selected_workspace_id
        .as_deref()
        .and_then(normalized_workspace_id);
    Ok(preferences)
}

fn write_preferences(path: &Path, preferences: &NativePreferences) -> Result<(), BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| BridgeError::Io(error.to_string()))?;
    }
    let body =
        serde_json::to_vec(preferences).map_err(|error| BridgeError::Io(error.to_string()))?;
    if body.len() > MAX_PREFERENCES_BYTES as usize {
        return Err(BridgeError::Io(
            "native preferences file is too large".into(),
        ));
    }
    std::fs::write(path, body).map_err(|error| BridgeError::Io(error.to_string()))
}

pub fn selected_workspace_id() -> Result<Option<String>, BridgeError> {
    let _guard = preferences_lock()
        .lock()
        .map_err(|_| BridgeError::Io("native preferences lock is unavailable".into()))?;
    Ok(read_preferences(&preferences_path()?)?.selected_workspace_id)
}

pub fn set_selected_workspace_id(value: Option<&str>) -> Result<(), BridgeError> {
    let normalized = match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(
            normalized_workspace_id(value)
                .ok_or_else(|| BridgeError::Io("selected workspace id must be a UUID".into()))?,
        ),
        None => None,
    };
    let _guard = preferences_lock()
        .lock()
        .map_err(|_| BridgeError::Io("native preferences lock is unavailable".into()))?;
    write_preferences(
        &preferences_path()?,
        &NativePreferences {
            selected_workspace_id: normalized,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_preference_roundtrips_and_normalizes_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        write_preferences(
            &path,
            &NativePreferences {
                selected_workspace_id: Some("11111111-1111-1111-1111-111111111111".into()),
            },
        )
        .unwrap();
        assert_eq!(
            read_preferences(&path)
                .unwrap()
                .selected_workspace_id
                .as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn invalid_or_oversized_preferences_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        std::fs::write(&path, br#"{"selected_workspace_id":"not-a-uuid"}"#).unwrap();
        assert_eq!(read_preferences(&path).unwrap().selected_workspace_id, None);

        std::fs::write(&path, vec![b'x'; MAX_PREFERENCES_BYTES as usize + 1]).unwrap();
        assert!(read_preferences(&path).is_err());
    }

    #[test]
    fn workspace_id_parser_rejects_unbounded_or_malformed_values() {
        assert!(normalized_workspace_id("not-a-uuid").is_none());
        assert!(normalized_workspace_id(&"a".repeat(20_000)).is_none());
    }
}
