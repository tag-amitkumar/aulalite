// crates/backend/src/services/file_assets.rs
//! Pure-function helpers for the upload pipeline: filename sanitization,
//! object-key formatting, request validation. No IO, no clock dependency
//! beyond what callers pass in explicitly.

use chrono::{DateTime, Datelike, Utc};
use uuid::Uuid;

fn truncate_to_byte_boundary(s: &str, max_bytes: usize) -> &str {
    let mut idx = max_bytes.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidateError {
    #[error("content type {0} not allowed for purpose {1}")]
    ContentTypeNotAllowed(String, String),
    #[error("size {0} exceeds {1} cap for purpose {2}")]
    SizeOverCap(i64, i64, String),
    #[error("size must be non-negative")]
    SizeNegative,
    #[error("unknown purpose: {0}")]
    UnknownPurpose(String),
}

const COVER_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp"];
const VIDEO_TYPES: &[&str] = &["video/mp4", "video/webm"];
const ATTACHMENT_TYPES: &[&str] = &[
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/zip",
    "text/plain",
    "text/csv",
    "image/jpeg",
    "image/png",
    "image/webp",
    "audio/mpeg",
    "video/mp4",
];

pub const COVER_MAX_SIZE: i64 = 5 * 1024 * 1024;
pub const VIDEO_MAX_SIZE: i64 = 500 * 1024 * 1024;
pub const ATTACHMENT_MAX_SIZE: i64 = 100 * 1024 * 1024;
/// Cap for a whiteboard paste/dropped image (matches the client-side limit).
pub const WHITEBOARD_IMAGE_MAX_SIZE: i64 = 8 * 1024 * 1024;
/// SCORM .zip package upload cap.
pub const SCORM_MAX_SIZE: i64 = 500 * 1024 * 1024;
/// Content types accepted for a SCORM package upload.
pub const SCORM_TYPES: &[&str] = &["application/zip", "application/x-zip-compressed"];

pub fn validate_request(
    purpose: &str,
    content_type: &str,
    size_bytes: i64,
) -> Result<(), ValidateError> {
    if size_bytes < 0 {
        return Err(ValidateError::SizeNegative);
    }
    let content_type = content_type.trim();
    let (allowed, cap) = match purpose {
        "cover" => (COVER_TYPES, COVER_MAX_SIZE),
        "video" => (VIDEO_TYPES, VIDEO_MAX_SIZE),
        "attachment" => (ATTACHMENT_TYPES, ATTACHMENT_MAX_SIZE),
        // Whiteboard paste/drop images: PNG/JPEG/WebP only, modest cap.
        "whiteboard" => (COVER_TYPES, WHITEBOARD_IMAGE_MAX_SIZE),
        // SCORM package .zip upload.
        "scorm" => (SCORM_TYPES, SCORM_MAX_SIZE),
        other => return Err(ValidateError::UnknownPurpose(other.to_string())),
    };
    if !allowed.iter().any(|t| t.eq_ignore_ascii_case(content_type)) {
        return Err(ValidateError::ContentTypeNotAllowed(
            content_type.to_string(),
            purpose.to_string(),
        ));
    }
    if size_bytes > cap {
        return Err(ValidateError::SizeOverCap(
            size_bytes,
            cap,
            purpose.to_string(),
        ));
    }
    Ok(())
}

pub fn sanitize_filename(input: &str) -> String {
    // Map every "bad" character (path separators, control chars, whitespace) to '_'.
    // Then collapse consecutive underscores to one, remove any underscore that is
    // immediately adjacent to a dot (e.g. "hi_.txt" → "hi.txt"), strip leading/trailing
    // underscores and leading dots, truncate to 200 chars preserving the extension.
    // Fall back to "unnamed" if the result is empty.

    // Step 1: map bad chars to '_'
    let mapped: String = input
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | '\0'..='\x1f' | '\x7f' | ' ' => '_',
            c => c,
        })
        .collect();

    // Step 2: collapse consecutive underscores into one
    let mut buf = String::with_capacity(mapped.len());
    let mut prev_underscore = false;
    for ch in mapped.chars() {
        if ch == '_' {
            if !prev_underscore {
                buf.push('_');
            }
            prev_underscore = true;
        } else {
            buf.push(ch);
            prev_underscore = false;
        }
    }

    // Step 3: remove underscores adjacent to dots (e.g. "hi_.txt" → "hi.txt", ".._" → "")
    // Repeatedly remove '_' immediately before or after a '.' until stable.
    loop {
        let new = buf.replace("_.", ".").replace("._", ".");
        if new == buf {
            break;
        }
        buf = new;
    }

    // Step 4: strip leading non-word chars (underscores and dots used as path traversal)
    let trimmed = buf.trim_start_matches(['_', '.']);
    let trimmed = trimmed.trim_end_matches('_');
    let buf = trimmed.to_string();

    if buf.is_empty() {
        return "unnamed".to_string();
    }

    // Step 5: truncate to 200 chars while preserving extension
    if buf.len() <= 200 {
        return buf;
    }
    if let Some(dot) = buf.rfind('.') {
        let ext = &buf[dot..];
        if ext.len() <= 16 {
            let prefix_len = 200 - ext.len();
            return format!(
                "{}{}",
                truncate_to_byte_boundary(&buf, prefix_len.min(dot)),
                ext
            );
        }
    }
    truncate_to_byte_boundary(&buf, 200).to_string()
}

pub fn object_key(tenant_id: Uuid, asset_id: Uuid, filename: &str, now: DateTime<Utc>) -> String {
    let sanitized = sanitize_filename(filename);
    format!(
        "{}/{:04}/{:02}/{}/{}",
        tenant_id.simple(),
        now.year(),
        now.month(),
        asset_id.simple(),
        sanitized
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_filename("../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_filename("a\\b\\c.txt"), "a_b_c.txt");
    }

    #[test]
    fn sanitize_strips_control_chars() {
        assert_eq!(sanitize_filename("hi\x00.txt"), "hi.txt");
        assert_eq!(sanitize_filename("a\nb.txt"), "a_b.txt");
    }

    #[test]
    fn sanitize_collapses_spaces_to_underscore() {
        assert_eq!(
            sanitize_filename("Week 1 Slides.pptx"),
            "Week_1_Slides.pptx"
        );
    }

    #[test]
    fn sanitize_preserves_extension_on_truncate() {
        let long = "x".repeat(300);
        let with_ext = format!("{long}.pdf");
        let out = sanitize_filename(&with_ext);
        assert!(out.len() <= 200);
        assert!(out.ends_with(".pdf"));
    }

    #[test]
    fn sanitize_handles_empty() {
        assert_eq!(sanitize_filename(""), "unnamed");
        assert_eq!(sanitize_filename("   "), "unnamed");
        assert_eq!(sanitize_filename("///"), "unnamed");
    }

    #[test]
    fn sanitize_truncates_at_char_boundary_for_multibyte() {
        // "学" is 3 bytes in UTF-8. 70 × 3 = 210 bytes — exceeds the 200-byte cap.
        // The plan extension-preserving path uses 200 - ext.len() = 196 byte budget,
        // and 196 / 3 = 65.33 — so we must end on a char boundary at byte 195 (65 chars).
        let stem: String = "学".repeat(70);
        let with_ext = format!("{stem}.pdf");
        let out = sanitize_filename(&with_ext);
        assert!(out.len() <= 200, "len was {}", out.len());
        assert!(out.ends_with(".pdf"));
        // No panic == primary success criterion. Output must be valid UTF-8.
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn sanitize_truncates_at_char_boundary_no_ext() {
        // 67 × 3 = 201 bytes, no extension — falls through to the final `[..200]` branch.
        let s: String = "学".repeat(67);
        let out = sanitize_filename(&s);
        assert!(out.len() <= 200);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn validate_trims_whitespace_in_content_type() {
        assert!(validate_request("cover", "  image/jpeg  ", 1000).is_ok());
    }

    #[test]
    fn object_key_format() {
        let tenant = Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap();
        let asset = Uuid::parse_str("4f1a2c8b-9d6e-4a7f-8c5b-3d2e1f9a0c8e").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 12, 0, 0).unwrap();
        let key = object_key(tenant, asset, "Slides.pptx", now);
        assert_eq!(
            key,
            "9c2f4a8e7b134f7c91d2b6a8e5c0d3e1/2026/05/4f1a2c8b9d6e4a7f8c5b3d2e1f9a0c8e/Slides.pptx"
        );
    }

    #[test]
    fn object_key_zero_pads_month() {
        let tenant = Uuid::nil();
        let asset = Uuid::nil();
        let now = Utc.with_ymd_and_hms(2026, 1, 8, 0, 0, 0).unwrap();
        let key = object_key(tenant, asset, "x.txt", now);
        assert!(key.contains("/2026/01/"));
    }

    #[test]
    fn validate_cover_accepts_jpeg() {
        assert!(validate_request("cover", "image/jpeg", 1_000_000).is_ok());
    }

    #[test]
    fn validate_cover_rejects_svg() {
        let err = validate_request("cover", "image/svg+xml", 1000).unwrap_err();
        assert_eq!(
            err,
            ValidateError::ContentTypeNotAllowed("image/svg+xml".into(), "cover".into())
        );
    }

    #[test]
    fn validate_cover_rejects_oversized() {
        let err = validate_request("cover", "image/png", 6_000_000).unwrap_err();
        assert_eq!(
            err,
            ValidateError::SizeOverCap(6_000_000, 5_242_880, "cover".into())
        );
    }

    #[test]
    fn validate_video_accepts_mp4() {
        assert!(validate_request("video", "video/mp4", 400_000_000).is_ok());
    }

    #[test]
    fn validate_attachment_accepts_pdf_under_cap() {
        assert!(validate_request("attachment", "application/pdf", 50_000_000).is_ok());
    }

    #[test]
    fn validate_attachment_rejects_executable() {
        let err = validate_request("attachment", "application/x-msdownload", 1000).unwrap_err();
        assert!(matches!(err, ValidateError::ContentTypeNotAllowed(_, _)));
    }

    #[test]
    fn validate_unknown_purpose() {
        let err = validate_request("avatar", "image/png", 1000).unwrap_err();
        assert_eq!(err, ValidateError::UnknownPurpose("avatar".into()));
    }

    #[test]
    fn validate_negative_size() {
        let err = validate_request("cover", "image/png", -1).unwrap_err();
        assert_eq!(err, ValidateError::SizeNegative);
    }
}
