// crates/backend/src/services/totp.rs
//! RFC 6238 TOTP (Time-based One-Time Password) + RFC 4648 base32, with no new
//! heavy dependencies.
//!
//! The HMAC-SHA1 core (RFC 2104 / RFC 4226 HOTP) is hand-rolled on top of the
//! `sha1` crate's one-shot `Sha1::digest` (a pure-Rust, C-free RustCrypto impl)
//! rather than the `hmac` generic wrapper, to keep the type plumbing trivial and
//! the surface auditable. TOTP (RFC 6238) is then HOTP keyed by the time-step
//! counter `floor(unix_time / period)`.
//!
//! Verification accepts a ±`window` step skew (default ±1 = ±30 s) to tolerate
//! client/server clock drift, and compares in constant time.
//!
//! Secrets are raw bytes; we expose base32 (no padding, uppercase) encode/decode
//! for the `otpauth://` URI and the human-readable secret shown at enrollment.

use sha1::{Digest, Sha1};

/// Default TOTP period in seconds (RFC 6238 recommends 30).
pub const DEFAULT_PERIOD: u64 = 30;
/// Default number of digits in the code (RFC 6238 / Google Authenticator use 6).
pub const DEFAULT_DIGITS: u32 = 6;
/// Default verify skew window, in steps. ±1 step => tolerate ±`period` seconds.
pub const DEFAULT_WINDOW: i64 = 1;
/// Length (in raw bytes) of a freshly-generated secret. 20 bytes = 160 bits =
/// the SHA-1 block-aligned size recommended by RFC 4226 §4.
pub const SECRET_LEN: usize = 20;

const SHA1_BLOCK: usize = 64;

/// HMAC-SHA1 of `msg` under `key` (RFC 2104). Returns the 20-byte MAC.
fn hmac_sha1(key: &[u8], msg: &[u8]) -> [u8; 20] {
    // Keys longer than the block size are first hashed down (RFC 2104).
    let mut k = [0u8; SHA1_BLOCK];
    if key.len() > SHA1_BLOCK {
        let digest = Sha1::digest(key);
        k[..digest.len()].copy_from_slice(&digest);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; SHA1_BLOCK];
    let mut opad = [0x5cu8; SHA1_BLOCK];
    for i in 0..SHA1_BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha1::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();

    let mut outer = Sha1::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();

    let mut mac = [0u8; 20];
    mac.copy_from_slice(&out);
    mac
}

/// HOTP (RFC 4226): the dynamic-truncation code for `counter` under `secret`.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let mac = hmac_sha1(secret, &counter.to_be_bytes());
    // Dynamic truncation (RFC 4226 §5.3).
    let offset = (mac[19] & 0x0f) as usize;
    let bin = ((mac[offset] as u32 & 0x7f) << 24)
        | ((mac[offset + 1] as u32) << 16)
        | ((mac[offset + 2] as u32) << 8)
        | (mac[offset + 3] as u32);
    bin % 10u32.pow(digits)
}

/// The TOTP code (zero-padded to `digits`) for `secret` at `unix_seconds`.
pub fn totp_at(secret: &[u8], unix_seconds: u64, period: u64, digits: u32) -> String {
    let counter = unix_seconds / period.max(1);
    let code = hotp(secret, counter, digits);
    format!("{code:0width$}", width = digits as usize)
}

/// Current TOTP code with the standard parameters.
pub fn current_code(secret: &[u8]) -> String {
    totp_at(secret, now_unix(), DEFAULT_PERIOD, DEFAULT_DIGITS)
}

/// TOTP code at a signed Unix timestamp, using standard parameters.
/// Negative timestamps are treated as the Unix epoch.
pub fn code_at(secret: &[u8], unix_seconds: i64) -> String {
    let unix_seconds = u64::try_from(unix_seconds).unwrap_or(0);
    totp_at(secret, unix_seconds, DEFAULT_PERIOD, DEFAULT_DIGITS)
}

/// Verify `code` against `secret` at `unix_seconds`, accepting a ±`window`-step
/// skew. Constant-time per-candidate compare; returns the matched time-step so
/// callers can implement RFC 6238 §5.2 replay prevention (a consumed step must
/// never verify again). All candidates are evaluated and folded without
/// early-return so the loop's timing does not reveal which step matched; the
/// highest matching step wins.
/// Returns `None` on any mismatch. `code` is trimmed before comparison.
pub fn verify_at_matched(
    secret: &[u8],
    code: &str,
    unix_seconds: u64,
    period: u64,
    digits: u32,
    window: i64,
) -> Option<i64> {
    let presented = code.trim();
    if presented.len() != digits as usize || !presented.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let base = (unix_seconds / period.max(1)) as i64;
    let mut matched: Option<i64> = None;
    for step in -window..=window {
        let counter = base + step;
        if counter < 0 {
            continue;
        }
        let candidate = format!(
            "{code:0width$}",
            code = hotp(secret, counter as u64, digits),
            width = digits as usize
        );
        // Fold every candidate into the result without early-return so the
        // loop's timing does not reveal which step matched.
        if ct_eq(candidate.as_bytes(), presented.as_bytes()) {
            matched = Some(matched.map_or(counter, |m| m.max(counter)));
        }
    }
    matched
}

/// Verify a presented `code` against `secret` at `unix_seconds`, accepting a
/// ±`window`-step skew. Constant-time per-candidate compare; returns true on the
/// first matching step. `code` is trimmed of whitespace before comparison.
pub fn verify_at(
    secret: &[u8],
    code: &str,
    unix_seconds: u64,
    period: u64,
    digits: u32,
    window: i64,
) -> bool {
    verify_at_matched(secret, code, unix_seconds, period, digits, window).is_some()
}

/// Verify `code` against `secret` right now with the standard parameters,
/// returning the matched time-step for replay tracking.
pub fn verify_matched(secret: &[u8], code: &str) -> Option<i64> {
    verify_at_matched(
        secret,
        code,
        now_unix(),
        DEFAULT_PERIOD,
        DEFAULT_DIGITS,
        DEFAULT_WINDOW,
    )
}

/// Verify `code` against `secret` right now with the standard parameters.
pub fn verify(secret: &[u8], code: &str) -> bool {
    verify_matched(secret, code).is_some()
}

/// Generate a fresh `SECRET_LEN`-byte CSPRNG secret.
pub fn generate_secret() -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; SECRET_LEN];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

/// Build the `otpauth://totp/...` provisioning URI consumed by authenticator
/// apps (Google Authenticator, Authy, 1Password, …). `label` is usually the
/// account email; `issuer` the product name. Both are percent-encoded.
pub fn otpauth_uri(secret_b32: &str, issuer: &str, label: &str) -> String {
    let label_enc = percent_encode(label);
    let issuer_enc = percent_encode(issuer);
    format!(
        "otpauth://totp/{issuer_enc}:{label_enc}?secret={secret_b32}&issuer={issuer_enc}&algorithm=SHA1&digits={DEFAULT_DIGITS}&period={DEFAULT_PERIOD}"
    )
}

/// Now, in whole unix seconds.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Constant-time byte-slice equality (length-dependent branch only).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// RFC 4648 base32 (uppercase, no padding) — enough for otpauth secrets.
// ---------------------------------------------------------------------------

const B32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encode raw bytes as RFC 4648 base32 (uppercase, NO `=` padding — which is
/// what `otpauth://` URIs expect).
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(B32_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(B32_ALPHABET[idx] as char);
    }
    out
}

/// Decode an RFC 4648 base32 string (case-insensitive; spaces and `=` padding
/// tolerated). Returns `None` on an invalid character.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for ch in s.chars() {
        if ch == '=' || ch.is_whitespace() {
            continue;
        }
        let up = ch.to_ascii_uppercase();
        let val = B32_ALPHABET.iter().position(|&c| c as char == up)? as u32;
        buffer = (buffer << 5) | val;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// Minimal percent-encoding for otpauth URI label/issuer segments. Keeps the
/// RFC 3986 unreserved set; everything else (incl. `@`, `:`, space) is escaped.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrips() {
        let raw = b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09";
        let enc = base32_encode(raw);
        assert!(enc.chars().all(|c| B32_ALPHABET.contains(&(c as u8))));
        let dec = base32_decode(&enc).unwrap();
        assert_eq!(dec, raw);
    }

    #[test]
    fn base32_decode_is_case_and_space_insensitive() {
        let raw = b"hello world!!";
        let enc = base32_encode(raw);
        let lower = enc.to_lowercase();
        let spaced = format!("{} {}", &lower[..4], &lower[4..]);
        assert_eq!(base32_decode(&spaced).unwrap(), raw);
    }

    /// RFC 6238 Appendix B reference vector for the SHA-1 / 8-digit suite.
    /// Secret = ASCII "12345678901234567890" (20 bytes). At T=59s the 8-digit
    /// code is 94287082; truncating to 6 digits gives 287082.
    #[test]
    fn rfc6238_reference_vector_sha1() {
        let secret = b"12345678901234567890";
        assert_eq!(totp_at(secret, 59, 30, 8), "94287082");
        assert_eq!(totp_at(secret, 59, 30, 6), "287082");
        assert_eq!(totp_at(secret, 1111111109, 30, 8), "07081804");
        assert_eq!(totp_at(secret, 1234567890, 30, 8), "89005924");
    }

    #[test]
    fn verify_accepts_current_and_adjacent_steps() {
        let secret = b"12345678901234567890";
        // T=59 lands in step 1; the code for step 1 must verify within ±1 at
        // a time inside step 1, and also at the boundary of the prior step.
        let code = totp_at(secret, 59, 30, 6);
        assert!(verify_at(secret, &code, 59, 30, 6, 1));
        // 30s earlier is step 0: still within ±1 window.
        assert!(verify_at(secret, &code, 35, 30, 6, 1));
        // 90s later (step 3) is outside ±1: must fail.
        assert!(!verify_at(secret, &code, 119, 30, 6, 1));
    }

    #[test]
    fn verify_matched_reports_the_step_that_matched() {
        let secret = b"12345678901234567890";
        let code = totp_at(secret, 59, 30, 6); // step 1
        assert_eq!(verify_at_matched(secret, &code, 59, 30, 6, 1), Some(1));
        // Presented while the server clock sits one step behind (step 0):
        // still matches, and reports the drifted-forward step so replay
        // tracking can consume it.
        assert_eq!(verify_at_matched(secret, &code, 35, 30, 6, 1), Some(1));
        assert_eq!(verify_at_matched(secret, "000000", 59, 30, 6, 1), None);
    }

    #[test]
    fn verify_rejects_wrong_shape() {
        let secret = b"12345678901234567890";
        assert!(!verify_at(secret, "12345", 59, 30, 6, 1)); // too short
        assert!(!verify_at(secret, "abcdef", 59, 30, 6, 1)); // non-digit
        assert!(!verify_at(secret, "", 59, 30, 6, 1));
    }

    #[test]
    fn generate_secret_is_full_length_and_random() {
        let a = generate_secret();
        let b = generate_secret();
        assert_eq!(a.len(), SECRET_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn otpauth_uri_has_expected_params() {
        let uri = otpauth_uri("JBSWY3DPEHPK3PXP", "AulaLite", "user@example.com");
        assert!(uri.starts_with("otpauth://totp/AulaLite:user%40example.com?"));
        assert!(uri.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("issuer=AulaLite"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }
}
