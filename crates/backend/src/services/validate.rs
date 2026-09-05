// crates/backend/src/services/validate.rs
//! Bounded-length validators for inbound user text fields.
//!
//! These return `ApiError::Validation(<stable_code>)` so handlers can wrap an
//! inbound field in one line and get a consistent 422 with a machine-readable
//! reason code (the existing handlers already use codes like `title_required`
//! / `title_too_long` / `body_required` / `body_too_long`; these helpers
//! centralize that pattern so every surface enforces the same caps).
//!
//! All length checks are by Unicode scalar count (not bytes) so multibyte text
//! is never rejected early — matching `handlers::announcements`.
//!
//! Pairing with `super::sanitize`: a handler typically validates the RAW
//! inbound value first (to reject obviously-bad input with a clear code), then
//! sanitizes it for storage. The caps here are the *outer* bound; sanitize's
//! own `max_chars` should be `>=` these so sanitization never silently rewrites
//! a value the validator already accepted.

use crate::error::ApiError;

/// Default caps shared across surfaces. Kept as public consts so handlers and
/// the sanitize layer can reference the same numbers.
pub const MAX_TITLE_LEN: usize = 200;
pub const MAX_NAME_LEN: usize = 120;
pub const MAX_BODY_LEN: usize = 10_000;
/// Live in-class chat: short, high-frequency lines.
pub const MAX_CHAT_LEN: usize = 2_000;

/// Validate a required, single-line-ish title field: trimmed, non-empty, within
/// `max` chars. Returns the trimmed slice on success. Stable codes:
/// `title_required`, `title_too_long`.
pub fn title(raw: &str, max: usize) -> Result<&str, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::Validation("title_required".into()));
    }
    if t.chars().count() > max {
        return Err(ApiError::Validation("title_too_long".into()));
    }
    Ok(t)
}

/// Validate a required body/markdown field: trimmed, non-empty, within `max`
/// chars. Returns the trimmed slice. Codes: `body_required`, `body_too_long`.
pub fn body(raw: &str, max: usize) -> Result<&str, ApiError> {
    let b = raw.trim();
    if b.is_empty() {
        return Err(ApiError::Validation("body_required".into()));
    }
    if b.chars().count() > max {
        return Err(ApiError::Validation("body_too_long".into()));
    }
    Ok(b)
}

/// Validate a required name field (display names, poll options, etc.): trimmed,
/// non-empty, within `max` chars. Codes: `name_required`, `name_too_long`.
pub fn name(raw: &str, max: usize) -> Result<&str, ApiError> {
    let n = raw.trim();
    if n.is_empty() {
        return Err(ApiError::Validation("name_required".into()));
    }
    if n.chars().count() > max {
        return Err(ApiError::Validation("name_too_long".into()));
    }
    Ok(n)
}

/// Validate a live-chat message line: trimmed, non-empty, within `MAX_CHAT_LEN`
/// chars. Codes: `chat_empty`, `chat_too_long`. Used by the WS chat ingest,
/// which currently checks `body.is_empty() || body.len() > 2000` on raw bytes;
/// this swaps in a char-count cap + trim so multibyte lines aren't mis-bounded.
pub fn chat(raw: &str) -> Result<&str, ApiError> {
    let c = raw.trim();
    if c.is_empty() {
        return Err(ApiError::Validation("chat_empty".into()));
    }
    if c.chars().count() > MAX_CHAT_LEN {
        return Err(ApiError::Validation("chat_too_long".into()));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: ApiError) -> String {
        match e {
            ApiError::Validation(c) => c,
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn title_trims_and_accepts() {
        assert_eq!(title("  Hello  ", MAX_TITLE_LEN).unwrap(), "Hello");
    }

    #[test]
    fn title_rejects_empty_and_too_long() {
        assert_eq!(
            code(title("   ", MAX_TITLE_LEN).unwrap_err()),
            "title_required"
        );
        let long = "x".repeat(MAX_TITLE_LEN + 1);
        assert_eq!(
            code(title(&long, MAX_TITLE_LEN).unwrap_err()),
            "title_too_long"
        );
    }

    #[test]
    fn body_codes() {
        assert_eq!(code(body("", MAX_BODY_LEN).unwrap_err()), "body_required");
        let long = "x".repeat(MAX_BODY_LEN + 1);
        assert_eq!(
            code(body(&long, MAX_BODY_LEN).unwrap_err()),
            "body_too_long"
        );
        assert_eq!(body(" ok ", MAX_BODY_LEN).unwrap(), "ok");
    }

    #[test]
    fn name_codes() {
        assert_eq!(code(name("", MAX_NAME_LEN).unwrap_err()), "name_required");
        let long = "x".repeat(MAX_NAME_LEN + 1);
        assert_eq!(
            code(name(&long, MAX_NAME_LEN).unwrap_err()),
            "name_too_long"
        );
    }

    #[test]
    fn chat_codes_and_char_count() {
        assert_eq!(code(chat("   ").unwrap_err()), "chat_empty");
        // Multibyte: char-count cap, not byte-length.
        let ok = "é".repeat(MAX_CHAT_LEN);
        assert_eq!(chat(&ok).unwrap().chars().count(), MAX_CHAT_LEN);
        let too_long = "é".repeat(MAX_CHAT_LEN + 1);
        assert_eq!(code(chat(&too_long).unwrap_err()), "chat_too_long");
    }
}
