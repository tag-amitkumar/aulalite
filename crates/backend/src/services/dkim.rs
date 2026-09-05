// crates/backend/src/services/dkim.rs
//! RFC 6376 DKIM signing (relaxed/relaxed, rsa-sha256).
//!
//! WHY THIS EXISTS / WHEN IT IS USED
//! ---------------------------------
//! AulaLite sends transactional email through the Resend HTTP API
//! (`services::notifications::ResendEmailNotifier`). When you send via an ESP's
//! HTTP API, DKIM is applied by the ESP at the edge using keys published in
//! **DNS** — our code never sees or signs the raw MIME message. So on the
//! current send path there is NOTHING to sign here; DKIM is a DNS-configuration
//! task (see the `pending` notes in the task output / `docs`).
//!
//! This module is therefore a CORRECT, TESTED, READY-TO-USE signer for a
//! FUTURE direct-SMTP / raw-MIME send path. If we ever add an SMTP transport
//! (e.g. lettre) we sign the assembled message with [`Signer::sign`] and
//! prepend the returned `DKIM-Signature:` header. It is deliberately NOT wired
//! into the Resend path (doing so would be wrong — Resend re-signs).
//!
//! WHAT IS IMPLEMENTED
//! -------------------
//!   * relaxed header canonicalization (RFC 6376 §3.4.2)
//!   * relaxed body canonicalization (RFC 6376 §3.4.4)
//!   * the signed `DKIM-Signature` tag set (v/a/c/d/s/t/bh/h/b) per §3.5
//!   * RSA-SHA256 (`a=rsa-sha256`) signing of the canonicalized header block
//!
//! The signer is constructed from env:
//!   * `AULALITE_DKIM_PRIVATE_KEY` — PEM (PKCS#8 or PKCS#1) RSA private key
//!   * `AULALITE_DKIM_SELECTOR`    — the DNS selector (e.g. `aulalite`)
//!   * `AULALITE_DKIM_DOMAIN`      — the signing domain `d=` (e.g. `example.com`)
//!
//! Unit tests cover the canonicalization (the load-bearing, spec-exact part);
//! signing is exercised against an ephemeral key.

use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};

use base64::Engine;

/// The fixed ASN.1 DigestInfo prefix for SHA-256, prepended to the 32-byte
/// hash to form the EMSA-PKCS1-v1_5 encoded message (RFC 8017 §9.2 /
/// RFC 3447). Using this with `Pkcs1v15Sign::new_unprefixed` lets us sign
/// without depending on `sha2`'s optional `oid` feature being enabled in the
/// build graph (it currently is not), which keeps this module self-contained.
const SHA256_DIGEST_INFO_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

#[derive(Debug, thiserror::Error)]
pub enum DkimError {
    #[error("dkim not configured")]
    NotConfigured,
    #[error("invalid private key: {0}")]
    Key(String),
    #[error("missing required header for signing: {0}")]
    MissingHeader(String),
    #[error("rsa sign failed: {0}")]
    Sign(String),
}

/// A configured DKIM signer. Cheap to clone is NOT required (constructed once),
/// but holding the parsed key avoids re-parsing per message.
pub struct Signer {
    selector: String,
    domain: String,
    key: RsaPrivateKey,
}

/// One email header as (name, unfolded-value). `name` is the raw header field
/// name (case is irrelevant — relaxed canon lower-cases it); `value` is the
/// header value WITHOUT the trailing CRLF and WITHOUT the leading colon.
#[derive(Debug, Clone)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl Signer {
    /// Build a signer from explicit parts. Accepts a PEM private key in either
    /// PKCS#8 (`BEGIN PRIVATE KEY`) or PKCS#1 (`BEGIN RSA PRIVATE KEY`) form —
    /// DKIM keys ship in both, so we try PKCS#8 first and fall back to PKCS#1.
    pub fn new(
        selector: impl Into<String>,
        domain: impl Into<String>,
        private_key_pem: &str,
    ) -> Result<Self, DkimError> {
        let key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
            .map_err(|e| DkimError::Key(e.to_string()))?;
        Ok(Self {
            selector: selector.into(),
            domain: domain.into(),
            key,
        })
    }

    /// Construct from environment, or `None` when DKIM is not configured. All
    /// three of selector/domain/private-key must be present.
    pub fn from_env() -> Option<Self> {
        let selector = std::env::var("AULALITE_DKIM_SELECTOR").ok()?;
        let domain = std::env::var("AULALITE_DKIM_DOMAIN").ok()?;
        let pem = std::env::var("AULALITE_DKIM_PRIVATE_KEY").ok()?;
        if selector.trim().is_empty() || domain.trim().is_empty() || pem.trim().is_empty() {
            return None;
        }
        match Self::new(selector.trim(), domain.trim(), &pem) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "DKIM env present but key invalid; signing disabled");
                None
            }
        }
    }

    /// Produce the value of the `DKIM-Signature` header for the given message.
    ///
    /// `headers` are the message headers (order as they appear in the message);
    /// `signed_header_names` lists, in order, which headers to sign (the `h=`
    /// tag) — typically `["From", "To", "Subject", "Date", "Message-ID"]`. Each
    /// MUST exist in `headers`. `body` is the raw message body (the part after
    /// the blank line separating headers from body).
    ///
    /// The returned string is the full header value to place after
    /// `DKIM-Signature:` (it does NOT include the field name or a trailing
    /// CRLF). The signature (`b=`) is appended last with an empty `b=` included
    /// in the signed header block, per RFC 6376 §3.7.
    pub fn sign(
        &self,
        headers: &[Header],
        signed_header_names: &[&str],
        body: &str,
        timestamp: i64,
    ) -> Result<String, DkimError> {
        // 1. Body hash: relaxed body canon, SHA-256, base64.
        let canon_body = canonicalize_body_relaxed(body);
        let bh = {
            let mut h = Sha256::new();
            h.update(canon_body.as_bytes());
            base64::engine::general_purpose::STANDARD.encode(h.finalize())
        };

        // 2. The h= tag: colon-joined header names in signing order.
        let h_tag = signed_header_names.join(":");

        // 3. Assemble the DKIM-Signature tag set WITHOUT the b= value. Per
        //    §3.5 the b= tag must be present (empty) when computing the
        //    signature; the final value is appended afterwards.
        let dkim_header_value_unsigned = format!(
            "v=1; a=rsa-sha256; c=relaxed/relaxed; d={domain}; s={selector}; \
             t={timestamp}; bh={bh}; h={h}; b=",
            domain = self.domain,
            selector = self.selector,
            timestamp = timestamp,
            bh = bh,
            h = h_tag,
        );

        // 4. Build the canonicalized header block to sign: each signed header
        //    (relaxed-canon) in h= order, then the DKIM-Signature header itself
        //    (relaxed-canon, with the empty b=) and — critically — NO trailing
        //    CRLF after it (RFC 6376 §3.7).
        let mut signing_input = String::new();
        for name in signed_header_names {
            let header = find_header(headers, name)
                .ok_or_else(|| DkimError::MissingHeader((*name).to_string()))?;
            signing_input.push_str(&canonicalize_header_relaxed(&header.name, &header.value));
            signing_input.push_str("\r\n");
        }
        // The DKIM-Signature header line, canonicalized, with no trailing CRLF.
        signing_input.push_str(&canonicalize_header_relaxed(
            "DKIM-Signature",
            &dkim_header_value_unsigned,
        ));

        // 5. Hash the canonicalized header block, wrap it in the SHA-256
        //    DigestInfo, RSA PKCS#1 v1.5 sign it, and base64 the signature; the
        //    result is appended as the b= value. `new_unprefixed` is used
        //    because we built the DigestInfo ourselves (see the prefix const).
        let mut hasher = Sha256::new();
        hasher.update(signing_input.as_bytes());
        let digest = hasher.finalize();
        let mut digest_info = Vec::with_capacity(SHA256_DIGEST_INFO_PREFIX.len() + digest.len());
        digest_info.extend_from_slice(&SHA256_DIGEST_INFO_PREFIX);
        digest_info.extend_from_slice(&digest);

        let signature = self
            .key
            .sign(Pkcs1v15Sign::new_unprefixed(), &digest_info)
            .map_err(|e| DkimError::Sign(e.to_string()))?;
        let b = base64::engine::general_purpose::STANDARD.encode(signature);

        Ok(format!("{dkim_header_value_unsigned}{b}"))
    }
}

/// Locate a header by case-insensitive name.
fn find_header<'a>(headers: &'a [Header], name: &str) -> Option<&'a Header> {
    headers.iter().find(|h| h.name.eq_ignore_ascii_case(name))
}

/// Relaxed HEADER canonicalization for a single header (RFC 6376 §3.4.2):
///   * field name lower-cased;
///   * remove whitespace before the colon between name and value;
///   * unfold the value (any CRLF that is part of folding is removed);
///   * collapse sequences of WSP (space/tab) within the value to a single SP;
///   * strip leading/trailing WSP from the value;
///   * output `name:value` (no trailing CRLF — the caller adds it).
fn canonicalize_header_relaxed(name: &str, value: &str) -> String {
    let lower_name = name.trim().to_ascii_lowercase();
    let canon_value = collapse_wsp_unfold(value);
    format!("{lower_name}:{canon_value}")
}

/// Unfold (remove CRLF used for folding) and collapse runs of WSP to one SP,
/// then trim. Used by relaxed header canonicalization for the value.
fn collapse_wsp_unfold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_wsp = false;
    for c in value.chars() {
        match c {
            // CR/LF are folding artifacts in a header value; drop them. Any
            // surrounding WSP collapses to a single SP via the in_wsp logic.
            '\r' | '\n' => {
                in_wsp = true;
            }
            ' ' | '\t' => {
                in_wsp = true;
            }
            other => {
                if in_wsp && !out.is_empty() {
                    out.push(' ');
                }
                in_wsp = false;
                out.push(other);
            }
        }
    }
    // Trailing WSP is dropped by construction (we only emit a space before a
    // non-WSP char); leading WSP never emitted because out is empty. Trim is a
    // belt-and-braces no-op for safety.
    out.trim().to_string()
}

/// Relaxed BODY canonicalization (RFC 6376 §3.4.4):
///   * reduce sequences of WSP within a line to a single SP;
///   * strip trailing WSP at the end of each line;
///   * normalize line endings to CRLF;
///   * remove all trailing empty lines, then ensure the body ends with a single
///     CRLF (a non-empty body); an empty body canonicalizes to a single CRLF.
fn canonicalize_body_relaxed(body: &str) -> String {
    // Normalize CRLF/CR/LF to \n for line splitting, then re-emit CRLF.
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = normalized
        .split('\n')
        .map(|line| {
            // Collapse interior WSP runs to a single SP and trim trailing WSP.
            let mut out = String::with_capacity(line.len());
            let mut in_wsp = false;
            for c in line.chars() {
                match c {
                    ' ' | '\t' => in_wsp = true,
                    other => {
                        // Any preceding WSP run (incl. a LEADING one) collapses
                        // to a single SP per RFC 6376 §3.4.4(b); only trailing
                        // WSP is stripped (handled by never emitting at line end).
                        if in_wsp {
                            out.push(' ');
                        }
                        in_wsp = false;
                        out.push(other);
                    }
                }
            }
            // `out` already has trailing WSP stripped (we never emit a trailing
            // space) and leading WSP collapsed only between non-empty content.
            // But a line of pure WSP would yield "" with in_wsp true — correct.
            out
        })
        .collect();

    // Remove trailing empty lines.
    while matches!(lines.last(), Some(l) if l.is_empty()) {
        lines.pop();
    }

    if lines.is_empty() {
        // Empty body canonicalizes to a single CRLF.
        return "\r\n".to_string();
    }

    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- header canonicalization (RFC 6376 §3.4.2) -------------------------

    #[test]
    fn header_relaxed_lowercases_name_and_collapses_wsp() {
        // From RFC 6376 §3.4.5 example spirit: "A: X" -> "a:X".
        assert_eq!(canonicalize_header_relaxed("A", "X"), "a:X");
    }

    #[test]
    fn header_relaxed_collapses_internal_wsp_to_single_space() {
        assert_eq!(
            canonicalize_header_relaxed("Subject", "hello   there\tworld"),
            "subject:hello there world"
        );
    }

    #[test]
    fn header_relaxed_strips_leading_and_trailing_wsp() {
        assert_eq!(
            canonicalize_header_relaxed("X-Test", "   value   "),
            "x-test:value"
        );
    }

    #[test]
    fn header_relaxed_unfolds_folded_value() {
        // A folded header value (CRLF + leading WSP on the continuation line)
        // becomes a single space, and the run collapses to one SP.
        assert_eq!(
            canonicalize_header_relaxed("Subject", "this is a\r\n folded subject"),
            "subject:this is a folded subject"
        );
    }

    #[test]
    fn header_relaxed_name_is_trimmed_and_lowercased() {
        assert_eq!(canonicalize_header_relaxed("  From  ", "a@b"), "from:a@b");
    }

    // --- body canonicalization (RFC 6376 §3.4.4) ---------------------------

    #[test]
    fn body_relaxed_empty_is_single_crlf() {
        assert_eq!(canonicalize_body_relaxed(""), "\r\n");
    }

    #[test]
    fn body_relaxed_strips_trailing_empty_lines_and_adds_one_crlf() {
        // Trailing blank lines are removed; body ends in exactly one CRLF.
        assert_eq!(canonicalize_body_relaxed("Hi there\n\n\n"), "Hi there\r\n");
    }

    #[test]
    fn body_relaxed_collapses_internal_wsp_and_strips_trailing_wsp() {
        assert_eq!(
            canonicalize_body_relaxed("a  b  \nc\t d \n"),
            "a b\r\nc d\r\n"
        );
    }

    #[test]
    fn body_relaxed_normalizes_lone_lf_and_cr_to_crlf() {
        assert_eq!(canonicalize_body_relaxed("a\nb\rc"), "a\r\nb\r\nc\r\n");
    }

    #[test]
    fn body_relaxed_known_vector_single_line() {
        // " C \r\nD \t E\r\n\r\n\r\n" canonicalizes (relaxed) to
        // " C\r\nD E\r\n" per RFC 6376 §3.4.5.
        let input = " C \r\nD \t E\r\n\r\n\r\n";
        assert_eq!(canonicalize_body_relaxed(input), " C\r\nD E\r\n");
    }

    // --- signing (smoke test against an ephemeral key) ---------------------

    #[test]
    fn sign_produces_well_formed_header_value() {
        // Generate an ephemeral RSA key, build a signer directly from it.
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("keygen");
        let signer = Signer {
            selector: "sel".into(),
            domain: "example.com".into(),
            key,
        };
        let headers = vec![
            Header {
                name: "From".into(),
                value: "Aula <no-reply@example.com>".into(),
            },
            Header {
                name: "To".into(),
                value: "user@example.org".into(),
            },
            Header {
                name: "Subject".into(),
                value: "Hello".into(),
            },
        ];
        let value = signer
            .sign(
                &headers,
                &["From", "To", "Subject"],
                "Body line\n",
                1_700_000_000,
            )
            .expect("sign");

        // Tag set present and well-formed.
        assert!(
            value.starts_with("v=1; a=rsa-sha256; c=relaxed/relaxed;"),
            "{value}"
        );
        assert!(value.contains("d=example.com;"), "{value}");
        assert!(value.contains("s=sel;"), "{value}");
        assert!(value.contains("h=From:To:Subject;"), "{value}");
        assert!(value.contains("bh="), "{value}");
        // b= is non-empty (a base64 signature was appended).
        let b = value.rsplit("b=").next().unwrap();
        assert!(!b.is_empty(), "signature b= must be non-empty: {value}");
        assert!(
            base64::engine::general_purpose::STANDARD.decode(b).is_ok(),
            "b= must be valid base64: {b}"
        );
    }

    #[test]
    fn sign_errors_on_missing_signed_header() {
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("keygen");
        let signer = Signer {
            selector: "sel".into(),
            domain: "example.com".into(),
            key,
        };
        let headers = vec![Header {
            name: "From".into(),
            value: "a@b".into(),
        }];
        let err = signer
            .sign(&headers, &["From", "Subject"], "body", 0)
            .unwrap_err();
        assert!(matches!(err, DkimError::MissingHeader(h) if h == "Subject"));
    }
}
