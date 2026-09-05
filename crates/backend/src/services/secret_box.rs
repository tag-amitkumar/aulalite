//! Authenticated application-layer encryption for secrets stored in PostgreSQL.
//!
//! The current key lives only in `AULALITE_DATA_ENCRYPTION_KEY` (base64-encoded
//! 32 bytes). An optional previous key permits a controlled rotation window.
//! Existing plaintext rows remain readable for migration, while every write is
//! encrypted whenever a current key is configured. Production startup requires
//! the current key, so new production data never falls back to plaintext. The
//! process snapshots the key pair on first use; changing deployment secrets
//! therefore requires the restart that a safe rotation already mandates.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use std::sync::OnceLock;

const CURRENT_KEY_ENV: &str = "AULALITE_DATA_ENCRYPTION_KEY";
const PREVIOUS_KEY_ENV: &str = "AULALITE_DATA_ENCRYPTION_KEY_PREVIOUS";
const BINARY_PREFIX: &[u8] = b"aula.enc.v1\0";
const TEXT_PREFIX: &str = "aula.enc.v1:";
const NONCE_LEN: usize = 12;
static KEY_CONFIGURATION: OnceLock<Result<KeyConfiguration, SecretBoxError>> = OnceLock::new();

#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretBoxError {
    #[error("{0} must be base64-encoded 32-byte key material")]
    InvalidKey(&'static str),
    #[error("{CURRENT_KEY_ENV} is required")]
    MissingCurrentKey,
    #[error("{PREVIOUS_KEY_ENV} cannot be set without {CURRENT_KEY_ENV}")]
    PreviousKeyWithoutCurrent,
    #[error("{PREVIOUS_KEY_ENV} must differ from {CURRENT_KEY_ENV}")]
    DuplicateRotationKey,
    #[error("encrypted secret cannot be opened without a configured key")]
    MissingDecryptionKey,
    #[error("encrypted secret payload is malformed")]
    MalformedPayload,
    #[error("secure random generation failed")]
    Random,
    #[error("secret encryption failed")]
    Encrypt,
    #[error("secret authentication failed")]
    Decrypt,
}

/// Fail-fast production configuration check. Also validates an optional
/// previous key so a typo cannot silently make old ciphertext unreadable.
pub fn validate_required_key() -> Result<(), SecretBoxError> {
    if key_configuration()?.current.is_none() {
        return Err(SecretBoxError::MissingCurrentKey);
    }
    Ok(())
}

pub fn current_key_configured() -> Result<bool, SecretBoxError> {
    Ok(key_configuration()?.current.is_some())
}

pub fn seal_bytes(plaintext: &[u8], context: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    let Some(key) = key_configuration()?.current else {
        return Ok(plaintext.to_vec());
    };
    seal_with_key(plaintext, context, &key)
}

pub fn open_bytes(stored: &[u8], context: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    if !stored.starts_with(BINARY_PREFIX) {
        return Ok(stored.to_vec());
    }
    open_with_configuration(stored, context, key_configuration()?)
}

pub fn seal_text(plaintext: &str, context: &[u8]) -> Result<String, SecretBoxError> {
    let Some(key) = key_configuration()?.current else {
        return Ok(plaintext.to_string());
    };
    let sealed = seal_with_key(plaintext.as_bytes(), context, &key)?;
    Ok(format!("{TEXT_PREFIX}{}", URL_SAFE_NO_PAD.encode(sealed)))
}

pub fn open_text(stored: &str, context: &[u8]) -> Result<String, SecretBoxError> {
    let Some(encoded) = stored.strip_prefix(TEXT_PREFIX) else {
        return Ok(stored.to_string());
    };
    let sealed = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| SecretBoxError::MalformedPayload)?;
    let plaintext = open_with_configuration(&sealed, context, key_configuration()?)?;
    String::from_utf8(plaintext).map_err(|_| SecretBoxError::MalformedPayload)
}

/// Return ciphertext under the current key only when the stored value is
/// plaintext or was encrypted with the configured previous key. Intended for
/// a privileged startup migration after schema migrations have completed.
pub fn rewrap_bytes(stored: &[u8], context: &[u8]) -> Result<Option<Vec<u8>>, SecretBoxError> {
    let configuration = key_configuration()?;
    let Some(current_key) = configuration.current else {
        return Ok(None);
    };

    let plaintext = if stored.starts_with(BINARY_PREFIX) {
        if open_with_keys(stored, context, &[current_key]).is_ok() {
            return Ok(None);
        }
        open_with_configuration(stored, context, configuration)?
    } else {
        stored.to_vec()
    };
    seal_with_key(&plaintext, context, &current_key).map(Some)
}

pub fn rewrap_text(stored: &str, context: &[u8]) -> Result<Option<String>, SecretBoxError> {
    let configuration = key_configuration()?;
    let Some(current_key) = configuration.current else {
        return Ok(None);
    };

    let plaintext = if let Some(encoded) = stored.strip_prefix(TEXT_PREFIX) {
        let sealed = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| SecretBoxError::MalformedPayload)?;
        if open_with_keys(&sealed, context, &[current_key]).is_ok() {
            return Ok(None);
        }
        open_with_configuration(&sealed, context, configuration)?
    } else {
        stored.as_bytes().to_vec()
    };
    let sealed = seal_with_key(&plaintext, context, &current_key)?;
    Ok(Some(format!(
        "{TEXT_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(sealed)
    )))
}

#[derive(Clone, Copy)]
struct KeyConfiguration {
    current: Option<[u8; 32]>,
    previous: Option<[u8; 32]>,
}

fn key_configuration() -> Result<&'static KeyConfiguration, SecretBoxError> {
    match KEY_CONFIGURATION.get_or_init(read_key_configuration_from_env) {
        Ok(configuration) => Ok(configuration),
        Err(error) => Err(error.clone()),
    }
}

fn open_with_configuration(
    stored: &[u8],
    context: &[u8],
    configuration: &KeyConfiguration,
) -> Result<Vec<u8>, SecretBoxError> {
    match (configuration.current, configuration.previous) {
        (Some(current), Some(previous)) => open_with_keys(stored, context, &[current, previous]),
        (Some(current), None) => open_with_keys(stored, context, &[current]),
        (None, None) => Err(SecretBoxError::MissingDecryptionKey),
        (None, Some(_)) => Err(SecretBoxError::PreviousKeyWithoutCurrent),
    }
}

fn read_key_configuration_from_env() -> Result<KeyConfiguration, SecretBoxError> {
    validate_key_configuration(read_key(CURRENT_KEY_ENV)?, read_key(PREVIOUS_KEY_ENV)?)
}

fn validate_key_configuration(
    current: Option<[u8; 32]>,
    previous: Option<[u8; 32]>,
) -> Result<KeyConfiguration, SecretBoxError> {
    match (current, previous) {
        (None, Some(_)) => Err(SecretBoxError::PreviousKeyWithoutCurrent),
        (Some(current), Some(previous)) if current == previous => {
            Err(SecretBoxError::DuplicateRotationKey)
        }
        (current, previous) => Ok(KeyConfiguration { current, previous }),
    }
}

fn read_key(name: &'static str) -> Result<Option<[u8; 32]>, SecretBoxError> {
    let Some(encoded) = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let decoded = URL_SAFE_NO_PAD
        .decode(&encoded)
        .or_else(|_| STANDARD.decode(&encoded))
        .or_else(|_| STANDARD_NO_PAD.decode(&encoded))
        .map_err(|_| SecretBoxError::InvalidKey(name))?;
    decoded
        .try_into()
        .map(Some)
        .map_err(|_| SecretBoxError::InvalidKey(name))
}

fn seal_with_key(
    plaintext: &[u8],
    context: &[u8],
    key_bytes: &[u8; 32],
) -> Result<Vec<u8>, SecretBoxError> {
    let key = LessSafeKey::new(
        UnboundKey::new(&aead::AES_256_GCM, key_bytes).map_err(|_| SecretBoxError::Encrypt)?,
    );
    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| SecretBoxError::Random)?;

    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::from(context),
        &mut ciphertext,
    )
    .map_err(|_| SecretBoxError::Encrypt)?;

    let mut output = Vec::with_capacity(BINARY_PREFIX.len() + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(BINARY_PREFIX);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn open_with_keys(
    stored: &[u8],
    context: &[u8],
    key_bytes: &[[u8; 32]],
) -> Result<Vec<u8>, SecretBoxError> {
    let payload = stored
        .strip_prefix(BINARY_PREFIX)
        .ok_or(SecretBoxError::MalformedPayload)?;
    if payload.len() < NONCE_LEN + aead::AES_256_GCM.tag_len() {
        return Err(SecretBoxError::MalformedPayload);
    }
    let (nonce, ciphertext) = payload.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| SecretBoxError::MalformedPayload)?;

    for key_bytes in key_bytes {
        let key = LessSafeKey::new(
            UnboundKey::new(&aead::AES_256_GCM, key_bytes).map_err(|_| SecretBoxError::Decrypt)?,
        );
        let mut candidate = ciphertext.to_vec();
        if let Ok(plaintext) = key.open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(context),
            &mut candidate,
        ) {
            return Ok(plaintext.to_vec());
        }
    }
    Err(SecretBoxError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::{
        open_with_keys, seal_with_key, validate_key_configuration, SecretBoxError, BINARY_PREFIX,
    };

    #[test]
    fn ciphertext_round_trips_and_is_context_bound() {
        let key = [7u8; 32];
        let sealed = seal_with_key(b"sensitive", b"tenant-a", &key).unwrap();

        assert!(sealed.starts_with(BINARY_PREFIX));
        assert!(!sealed.windows(9).any(|window| window == b"sensitive"));
        assert_eq!(
            open_with_keys(&sealed, b"tenant-a", &[key]).unwrap(),
            b"sensitive"
        );
        assert!(open_with_keys(&sealed, b"tenant-b", &[key]).is_err());
    }

    #[test]
    fn previous_key_can_open_during_rotation() {
        let old_key = [4u8; 32];
        let new_key = [5u8; 32];
        let sealed = seal_with_key(b"secret", b"scope", &old_key).unwrap();

        assert_eq!(
            open_with_keys(&sealed, b"scope", &[new_key, old_key]).unwrap(),
            b"secret"
        );
    }

    #[test]
    fn tampering_is_rejected() {
        let key = [9u8; 32];
        let mut sealed = seal_with_key(b"secret", b"scope", &key).unwrap();
        *sealed.last_mut().unwrap() ^= 1;

        assert!(open_with_keys(&sealed, b"scope", &[key]).is_err());
    }

    #[test]
    fn rotation_key_configuration_is_fail_safe() {
        assert!(matches!(
            validate_key_configuration(None, Some([1u8; 32])),
            Err(SecretBoxError::PreviousKeyWithoutCurrent)
        ));
        assert!(matches!(
            validate_key_configuration(Some([2u8; 32]), Some([2u8; 32])),
            Err(SecretBoxError::DuplicateRotationKey)
        ));
        assert!(validate_key_configuration(Some([2u8; 32]), Some([3u8; 32])).is_ok());
    }

    #[test]
    fn truncated_ciphertext_is_rejected_before_aead_open() {
        let mut malformed = BINARY_PREFIX.to_vec();
        malformed.extend_from_slice(&[0u8; 12]);
        assert!(matches!(
            open_with_keys(&malformed, b"scope", &[[4u8; 32]]),
            Err(SecretBoxError::MalformedPayload)
        ));
    }
}
