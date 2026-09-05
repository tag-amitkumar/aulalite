//! Bounded native read cache and an explicit mutation-sync safety contract.
//! Authentication material is never stored; callers partition keys with a
//! one-way principal hash and clear the cache on sign-out.

#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u8 = 1;
const MAX_ENTRY_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENVELOPE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    ReadThrough {
        fresh_for_secs: u64,
        stale_if_offline_secs: u64,
    },
    /// A future endpoint may opt in only after backend idempotency-key and
    /// conflict semantics exist. No current mutation is classified this way.
    IdempotentReplace,
    OnlineOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheFreshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheHit {
    pub bytes: Vec<u8>,
    pub freshness: CacheFreshness,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEnvelope {
    version: u8,
    key_hash: String,
    stored_at_unix: u64,
    fresh_until_unix: u64,
    stale_until_unix: u64,
    body: Vec<u8>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn cache_dir() -> Result<PathBuf, BridgeError> {
    Ok(crate::native_paths::cache_dir()?.join("http-v1"))
}

fn hash_hex(input: &[u8]) -> String {
    Sha256::digest(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn authenticated_principal() -> &'static RwLock<Option<String>> {
    static PRINCIPAL: OnceLock<RwLock<Option<String>>> = OnceLock::new();
    PRINCIPAL.get_or_init(|| RwLock::new(None))
}

/// Set only after `/v1/me` has authenticated the session. A stable backend user
/// id keeps cache ownership stable across token rotation without trusting an
/// unsigned claim from an incoming deep link.
pub fn set_authenticated_principal(user_id: &str) -> Result<(), BridgeError> {
    if user_id.trim().is_empty() || user_id.len() > 128 || user_id.chars().any(char::is_control) {
        return Err(BridgeError::Configuration(
            "The authenticated cache principal is invalid.".into(),
        ));
    }
    *authenticated_principal()
        .write()
        .map_err(|_| BridgeError::Io("offline principal unavailable".into()))? =
        Some(hash_hex(user_id.trim().as_bytes()));
    Ok(())
}

pub fn current_principal_partition() -> Option<String> {
    authenticated_principal().read().ok()?.clone()
}

fn clear_principal_marker() {
    if let Ok(mut principal) = authenticated_principal().write() {
        *principal = None;
    }
}

pub fn cache_key(workspace_id: Option<&str>, principal_hash: &str, path: &str) -> String {
    let resource_hash =
        hash_hex(format!("{}\n{}", workspace_id.unwrap_or("global"), path).as_bytes());
    format!("{principal_hash}_{resource_hash}")
}

fn entry_path(root: &Path, key_hash: &str) -> PathBuf {
    root.join(format!("{key_hash}.json"))
}

fn path_without_query(path: &str) -> &str {
    path.split_once('?').map(|(path, _)| path).unwrap_or(path)
}

/// Only low-risk learning reads are cached. Billing, auth, roles, admin,
/// grades, submissions, attendance, notifications, exports, and live state are
/// deliberately online-only even when they use GET.
pub fn sync_policy(method: &str, path: &str) -> SyncPolicy {
    if method != "GET" {
        return SyncPolicy::OnlineOnly;
    }
    let path = path_without_query(path);
    let denied = [
        "/admin",
        "/billing",
        "/grade",
        "/submission",
        "/attendance",
        "/notification",
        "/device-token",
        "/export",
        "/live",
        "/audit",
        "/member",
        "/seat",
        "/sso",
        "/lti",
        "/api-key",
        "/webhook",
    ];
    if denied.iter().any(|segment| path.contains(segment)) {
        return SyncPolicy::OnlineOnly;
    }
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    let low_risk_course_read = segments.as_slice() == ["v1", "courses"]
        || matches!(segments.as_slice(), ["v1", "courses", _])
        || path.contains("/modules")
        || path.contains("/lessons")
        || path.contains("/announcements")
        || path.contains("/flashcards");
    if low_risk_course_read {
        SyncPolicy::ReadThrough {
            fresh_for_secs: 15 * 60,
            stale_if_offline_secs: 7 * 24 * 60 * 60,
        }
    } else {
        SyncPolicy::OnlineOnly
    }
}

fn put_at(
    root: &Path,
    key_hash: &str,
    body: &[u8],
    fresh_for_secs: u64,
    stale_if_offline_secs: u64,
) -> Result<(), BridgeError> {
    if body.len() > MAX_ENTRY_BYTES {
        return Ok(());
    }
    std::fs::create_dir_all(root).map_err(|e| BridgeError::Io(e.to_string()))?;
    let now = now_unix();
    let envelope = CacheEnvelope {
        version: CACHE_VERSION,
        key_hash: key_hash.to_string(),
        stored_at_unix: now,
        fresh_until_unix: now.saturating_add(fresh_for_secs),
        stale_until_unix: now.saturating_add(stale_if_offline_secs),
        body: body.to_vec(),
    };
    let encoded = serde_json::to_vec(&envelope).map_err(|e| BridgeError::Io(e.to_string()))?;
    let path = entry_path(root, key_hash);
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, encoded).map_err(|e| BridgeError::Io(e.to_string()))?;
    #[cfg(windows)]
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(BridgeError::Io(error.to_string()));
        }
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(BridgeError::Io(error.to_string()));
    }
    prune_at(root, MAX_CACHE_BYTES)
}

pub fn put(
    key_hash: &str,
    body: &[u8],
    fresh_for_secs: u64,
    stale_if_offline_secs: u64,
) -> Result<(), BridgeError> {
    put_at(
        &cache_dir()?,
        key_hash,
        body,
        fresh_for_secs,
        stale_if_offline_secs,
    )
}

fn get_at(root: &Path, key_hash: &str, now: u64) -> Result<Option<CacheHit>, BridgeError> {
    let path = entry_path(root, key_hash);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BridgeError::Io(error.to_string())),
    };
    if metadata.len() > MAX_ENVELOPE_BYTES {
        let _ = std::fs::remove_file(path);
        return Ok(None);
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BridgeError::Io(error.to_string())),
    };
    let envelope: CacheEnvelope = match serde_json::from_slice(&bytes) {
        Ok(envelope) => envelope,
        Err(_) => {
            let _ = std::fs::remove_file(path);
            return Ok(None);
        }
    };
    if envelope.version != CACHE_VERSION
        || envelope.key_hash != key_hash
        || envelope.body.len() > MAX_ENTRY_BYTES
        || now > envelope.stale_until_unix
    {
        let _ = std::fs::remove_file(path);
        return Ok(None);
    }
    Ok(Some(CacheHit {
        bytes: envelope.body,
        freshness: if now <= envelope.fresh_until_unix {
            CacheFreshness::Fresh
        } else {
            CacheFreshness::Stale
        },
    }))
}

pub fn get(key_hash: &str) -> Result<Option<CacheHit>, BridgeError> {
    get_at(&cache_dir()?, key_hash, now_unix())
}

fn prune_at(root: &Path, quota: u64) -> Result<(), BridgeError> {
    let mut files = Vec::new();
    let mut total = 0_u64;
    for entry in match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(BridgeError::Io(error.to_string())),
    } {
        let entry = entry.map_err(|e| BridgeError::Io(e.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|e| BridgeError::Io(e.to_string()))?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
            files.push((metadata.modified().ok(), metadata.len(), entry.path()));
        }
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= quota {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

pub fn clear_all() -> Result<(), BridgeError> {
    let root = cache_dir()?;
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(BridgeError::Io(error.to_string())),
    }
}

pub fn clear_current_principal() -> Result<(), BridgeError> {
    let Some(principal) = current_principal_partition() else {
        return Ok(());
    };
    let root = cache_dir()?;
    match std::fs::read_dir(&root) {
        Ok(entries) => {
            let prefix = format!("{principal}_");
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(&prefix))
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(BridgeError::Io(error.to_string())),
    }
    clear_principal_marker();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_keeps_sensitive_and_mutating_requests_online() {
        assert!(matches!(
            sync_policy("GET", "/v1/courses"),
            SyncPolicy::ReadThrough { .. }
        ));
        for (method, path) in [
            ("POST", "/v1/courses"),
            ("GET", "/v1/admin/billing"),
            ("GET", "/v1/courses/math/gradebook"),
            ("GET", "/v1/me/export"),
            ("PATCH", "/v1/me/notification-preferences"),
        ] {
            assert_eq!(sync_policy(method, path), SyncPolicy::OnlineOnly, "{path}");
        }
    }

    #[test]
    fn cache_roundtrip_marks_fresh_and_stale_then_expires() {
        let dir = tempfile::tempdir().unwrap();
        put_at(dir.path(), "abc", b"course", 10, 30).unwrap();
        let stored: CacheEnvelope =
            serde_json::from_slice(&std::fs::read(entry_path(dir.path(), "abc")).unwrap()).unwrap();
        assert_eq!(
            get_at(dir.path(), "abc", stored.stored_at_unix + 5)
                .unwrap()
                .unwrap()
                .freshness,
            CacheFreshness::Fresh
        );
        assert_eq!(
            get_at(dir.path(), "abc", stored.stored_at_unix + 20)
                .unwrap()
                .unwrap()
                .freshness,
            CacheFreshness::Stale
        );
        assert!(get_at(dir.path(), "abc", stored.stored_at_unix + 31)
            .unwrap()
            .is_none());
    }

    #[test]
    fn cache_key_partitions_users_and_workspaces_without_storing_tokens() {
        let principal_a = hash_hex(b"user-a");
        let principal_b = hash_hex(b"user-b");
        assert_ne!(principal_a, principal_b);
        assert!(!principal_a.contains("user"));
        assert_ne!(
            cache_key(Some("workspace-a"), &principal_a, "/v1/courses"),
            cache_key(Some("workspace-b"), &principal_a, "/v1/courses")
        );
    }
}
