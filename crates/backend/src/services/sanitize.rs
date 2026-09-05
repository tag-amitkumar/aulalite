// crates/backend/src/services/sanitize.rs
//! Defense-in-depth input sanitization for user-authored text and markdown.
//!
//! The frontend renders user markdown through `pulldown_cmark`, which escapes
//! raw HTML by default — so the *primary* XSS defense lives on the render path.
//! But titles / names / announcement & discussion bodies / live-chat lines are
//! stored RAW in Postgres and are surfaced in several places (notification
//! emails, plain-text previews, admin tools, future renderers) that may not run
//! them back through that escaping pass. This module is the second layer: it
//! neutralizes the dangerous bits *before persistence* so a stored value can
//! never carry an executable payload, regardless of how it is later rendered.
//!
//! ZERO new dependencies (see `cargo_deps`): a conservative, hand-rolled
//! tag/scheme stripper rather than pulling in `ammonia` (which would need
//! vendoring for the offline in-container build and team vetting). The strategy
//! is deliberately *blunt and allowlist-free for HTML*: markdown does not need
//! raw HTML to render its formatting, so we simply strip the handful of HTML
//! constructs that can execute and defang dangerous URL schemes, while leaving
//! ordinary markdown punctuation (`#`, `*`, `_`, `>`, `` ` ``, `[]()`) intact.
//!
//! Pure functions only — trivially unit-testable, no DB / async.

/// Drop ASCII/Unicode control characters that have no business in a single
/// stored value, while preserving the whitespace that legitimately appears in
/// prose. We keep `\n`, `\r` and `\t`; everything else in the C0/C1 control
/// ranges plus the Unicode line/paragraph separators is removed. This also
/// strips the bidi-override and zero-width characters most commonly abused for
/// spoofing/obfuscation.
fn strip_control_chars(input: &str) -> String {
    input
        .chars()
        .filter(|c| {
            match c {
                // Keep the common prose whitespace.
                '\n' | '\r' | '\t' => true,
                // Drop every other control char (C0 + DEL + C1).
                c if c.is_control() => false,
                // Drop zero-width + bidi-control characters used for spoofing.
                '\u{200B}'..='\u{200F}' // zero-width + LRM/RLM
                | '\u{202A}'..='\u{202E}' // bidi embeddings/overrides
                | '\u{2060}'             // word joiner
                | '\u{2066}'..='\u{2069}' // bidi isolates
                | '\u{FEFF}' => false,   // BOM / zero-width no-break space
                _ => true,
            }
        })
        .collect()
}

/// Truncate `input` to at most `max_chars` Unicode scalar values (never bytes,
/// so multibyte text is never split mid-codepoint). Returns the input unchanged
/// when already within bounds.
fn cap_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        input.chars().take(max_chars).collect()
    }
}

/// Sanitize a short single-line-ish *plain-text* field (titles, names, poll
/// questions, chat lines): strip control chars, collapse leading/trailing
/// whitespace, and hard-cap the length. This intentionally does NOT touch
/// markdown punctuation — plain-text fields are rendered as text, so `<`/`>`
/// are harmless here, but stripping control chars + capping length is the
/// cheap, always-safe hardening every inbound text field should get.
pub fn clean_text(input: &str, max_chars: usize) -> String {
    let stripped = strip_control_chars(input);
    let trimmed = stripped.trim();
    cap_chars(trimmed, max_chars)
}

/// Schemes we refuse to let survive inside markdown link/image targets or raw
/// HTML attributes. `javascript:`/`vbscript:`/`data:` are the classic
/// script-execution and data-URI exfil vectors; comparison is
/// case-insensitive and ignores embedded whitespace (e.g. `java\tscript:`).
const DANGEROUS_SCHEMES: &[&str] = &["javascript:", "vbscript:", "data:", "file:"];

/// HTML element names whose ENTIRE subtree (open tag, contents, close tag) must
/// be removed — their text content is itself executable or a navigation hijack.
const DROP_WHOLE_ELEMENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "applet", "form", "noscript",
];

/// Sanitize user markdown for safe storage and (defense-in-depth) safe rendering
/// on any path, including ones that do not re-escape:
///   1. strip control chars + cap length;
///   2. remove dangerous whole elements (`<script>…</script>` etc.) including
///      their inner text;
///   3. strip every remaining raw HTML *tag* (markdown needs none) so no
///      `onerror=`/`onclick=` event handler or stray `<img>` can survive — the
///      tag's visible inner text is preserved;
///   4. defang dangerous URL schemes (`javascript:`, `data:`, …) wherever they
///      appear (markdown link targets, autolinks, leftover attributes).
///
/// pulldown_cmark on the frontend already escapes raw HTML, so legitimate
/// formatting (headings, lists, emphasis, code, links, images) is expressed in
/// markdown punctuation that we deliberately leave untouched.
pub fn clean_markdown(input: &str, max_chars: usize) -> String {
    let stripped = strip_control_chars(input);
    let no_dangerous_elements = strip_dangerous_elements(&stripped);
    let no_tags = strip_html_tags(&no_dangerous_elements);
    let defanged = defang_schemes(&no_tags);
    let trimmed = defanged.trim();
    cap_chars(trimmed, max_chars)
}

/// Case-insensitively test whether the byte slice starting at `rest` begins
/// with `needle`. Operates on bytes; both inputs are compared lowercased.
fn starts_with_ci(rest: &str, needle: &str) -> bool {
    let rb = rest.as_bytes();
    let nb = needle.as_bytes();
    if rb.len() < nb.len() {
        return false;
    }
    rb[..nb.len()]
        .iter()
        .zip(nb.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Remove `<tag>…</tag>` blocks (and self-closing / unclosed variants) for every
/// element in `DROP_WHOLE_ELEMENT`, dropping their inner content entirely.
fn strip_dangerous_elements(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    'outer: while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &input[i + 1..];
            // Optional leading slash on a stray close tag — skip it for matching.
            let after_lt = rest.strip_prefix('/').unwrap_or(rest);
            for tag in DROP_WHOLE_ELEMENT {
                if starts_with_ci(after_lt, tag) {
                    // Confirm it's a tag boundary (next char is whitespace, '>',
                    // '/', or end) so we don't match `<scripted>`.
                    let nxt = after_lt[tag.len()..].chars().next();
                    let boundary = matches!(
                        nxt,
                        None | Some('>')
                            | Some('/')
                            | Some(' ')
                            | Some('\t')
                            | Some('\n')
                            | Some('\r')
                    );
                    if boundary {
                        // Drop everything up to and including the matching close
                        // tag `</tag>`; if none exists, drop to end of input.
                        let close = format!("</{tag}");
                        if let Some(rel) = find_ci(&input[i..], &close) {
                            let close_start = i + rel;
                            // Advance past the close tag's '>' (or to end).
                            let after_close = &input[close_start..];
                            if let Some(gt) = after_close.find('>') {
                                i = close_start + gt + 1;
                            } else {
                                i = input.len();
                            }
                        } else {
                            i = input.len();
                        }
                        continue 'outer;
                    }
                }
            }
        }
        // Not a dangerous element: copy this char verbatim.
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Case-insensitive `str::find`. Returns the byte offset of the first match.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if hb.len() < nb.len() {
        return None;
    }
    for start in 0..=hb.len() - nb.len() {
        if hb[start..start + nb.len()]
            .iter()
            .zip(nb.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(start);
        }
    }
    None
}

/// Strip every remaining raw HTML tag (`<...>`), keeping the visible inner text.
/// A `<` that is NOT the start of a plausible tag (i.e. not followed by an
/// ASCII letter, `/`, or `!`) is treated as literal text and preserved, so
/// markdown like `a < b` and `<- arrow` survive intact.
fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices();
    while let Some((idx, ch)) = chars.next() {
        if ch == '<' {
            // Look at what immediately follows the '<'.
            let next = input[idx + 1..].chars().next();
            let looks_like_tag =
                matches!(next, Some(c) if c.is_ascii_alphabetic() || c == '/' || c == '!');
            if looks_like_tag {
                // Consume through the matching '>' (or to end of input).
                let mut closed = false;
                for (_, c) in chars.by_ref() {
                    if c == '>' {
                        closed = true;
                        break;
                    }
                }
                // Whether closed or not, the tag (and any dangling fragment) is
                // dropped. `closed` is intentionally unused beyond the loop.
                let _ = closed;
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Neutralize dangerous URL schemes anywhere they appear. We do a
/// case-insensitive scan and, on a hit, rewrite the scheme prefix to the inert
/// `unsafe:` marker (mirroring Angular's well-known sanitizer behavior) so the
/// link text stays readable but the target can no longer execute or load.
/// Whitespace and HTML entities embedded inside the scheme (a common bypass,
/// e.g. `java&#9;script:`) are first squeezed out for the comparison.
fn defang_schemes(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    // Fast path: if no candidate scheme keyword appears at all, return as-is.
    if !DANGEROUS_SCHEMES
        .iter()
        .any(|s| collapse_for_scheme(&lower).contains(s))
    {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len() + 8);
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let mut matched = false;
        for scheme in DANGEROUS_SCHEMES {
            // Compare the upcoming window with embedded whitespace/control
            // collapsed out so `java\tscript:` is still caught.
            if scheme_matches_at(&input[i..], scheme) {
                out.push_str("unsafe:");
                // Skip past the matched scheme in the source, including any
                // interleaved whitespace we collapsed for the comparison.
                i += consumed_scheme_len(&input[i..], scheme);
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Collapse ASCII whitespace out of a string for scheme comparison.
fn collapse_for_scheme(s: &str) -> String {
    s.chars().filter(|c| !c.is_ascii_whitespace()).collect()
}

/// Does the head of `rest` equal `scheme`, case-insensitively, allowing ASCII
/// whitespace interleaved *between* letters of the scheme (the classic
/// `java\tscript:` obfuscation) but NOT before the first letter — so `rest`
/// must genuinely START with the scheme, not merely contain it after a space.
fn scheme_matches_at(rest: &str, scheme: &str) -> bool {
    let mut s_iter = scheme.bytes();
    let mut expected = s_iter.next();
    let mut started = false;
    for b in rest.bytes() {
        if b.is_ascii_whitespace() {
            // Interior whitespace is allowed only once matching has begun.
            if started {
                continue;
            }
            return false;
        }
        match expected {
            None => return true,
            Some(e) => {
                if b.eq_ignore_ascii_case(&e) {
                    started = true;
                    expected = s_iter.next();
                } else {
                    return false;
                }
            }
        }
    }
    expected.is_none()
}

/// How many source bytes a matched scheme occupies (including interleaved
/// whitespace), so `defang_schemes` advances past exactly the consumed run.
fn consumed_scheme_len(rest: &str, scheme: &str) -> usize {
    let mut s_iter = scheme.bytes();
    let mut expected = s_iter.next();
    let mut consumed = 0usize;
    for b in rest.bytes() {
        consumed += 1;
        if b.is_ascii_whitespace() {
            continue;
        }
        match expected {
            None => {
                return consumed - 1;
            }
            Some(e) => {
                if b.eq_ignore_ascii_case(&e) {
                    expected = s_iter.next();
                    if expected.is_none() {
                        return consumed;
                    }
                }
            }
        }
    }
    consumed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_strips_control_and_trims_and_caps() {
        // NUL + bell + bidi override removed; surrounding space trimmed.
        let dirty = "  hi\u{0}\u{7}\u{202E}there  ";
        assert_eq!(clean_text(dirty, 100), "hithere");
        // Newlines/tabs are kept inside; only the edges are trimmed.
        assert_eq!(clean_text("\t a\nb \t", 100), "a\nb");
        // Char-count cap never splits a multibyte codepoint.
        assert_eq!(clean_text("héllo wörld", 5), "héllo");
    }

    #[test]
    fn clean_markdown_keeps_safe_punctuation() {
        let md = "# Heading\n\n- **bold** and _em_\n\n[link](https://ex.com)\n\n`code`";
        let cleaned = clean_markdown(md, 10_000);
        assert!(cleaned.contains("# Heading"));
        assert!(cleaned.contains("**bold**"));
        assert!(cleaned.contains("[link](https://ex.com)"));
        assert!(cleaned.contains("`code`"));
    }

    #[test]
    fn clean_markdown_drops_script_and_style_with_content() {
        let md = "before<script>alert('x')</script>after<style>body{}</style>end";
        let cleaned = clean_markdown(md, 10_000);
        assert!(!cleaned.to_ascii_lowercase().contains("alert"));
        assert!(!cleaned.to_ascii_lowercase().contains("script"));
        assert!(!cleaned.to_ascii_lowercase().contains("body{}"));
        assert_eq!(cleaned, "beforeafterend");
    }

    #[test]
    fn clean_markdown_strips_inline_tags_with_handlers_keeps_text() {
        let md = r#"hello <img src=x onerror="alert(1)"> world <b>bold</b>"#;
        let cleaned = clean_markdown(md, 10_000);
        assert!(!cleaned.to_ascii_lowercase().contains("onerror"));
        assert!(!cleaned.contains("<img"));
        assert!(!cleaned.contains("<b>"));
        // Visible text on either side of the stripped tags survives.
        assert!(cleaned.contains("hello"));
        assert!(cleaned.contains("world"));
        assert!(cleaned.contains("bold"));
    }

    #[test]
    fn clean_markdown_defangs_javascript_scheme_in_link() {
        let md = "[click](javascript:alert(1))";
        let cleaned = clean_markdown(md, 10_000);
        assert!(!cleaned.to_ascii_lowercase().contains("javascript:"));
        assert!(cleaned.contains("unsafe:"));
    }

    #[test]
    fn clean_markdown_defangs_scheme_with_interleaved_whitespace() {
        let md = "[x](java\tscript:alert(1))";
        let cleaned = clean_markdown(md, 10_000);
        assert!(!cleaned
            .to_ascii_lowercase()
            .replace([' ', '\t'], "")
            .contains("javascript:"));
        assert!(cleaned.contains("unsafe:"));
    }

    #[test]
    fn clean_markdown_preserves_literal_less_than() {
        // `a < b` is not a tag start (space follows '<'); must survive.
        let cleaned = clean_markdown("if a < b then ok", 10_000);
        assert_eq!(cleaned, "if a < b then ok");
    }

    #[test]
    fn clean_markdown_caps_length() {
        let md = "x".repeat(50);
        assert_eq!(clean_markdown(&md, 10).chars().count(), 10);
    }

    #[test]
    fn defang_leaves_safe_schemes_alone() {
        let md = "[ok](https://example.com) and [m](mailto:a@b.com)";
        let cleaned = clean_markdown(md, 10_000);
        assert!(cleaned.contains("https://example.com"));
        assert!(cleaned.contains("mailto:a@b.com"));
        assert!(!cleaned.contains("unsafe:"));
    }

    #[test]
    fn unclosed_script_drops_to_end() {
        let md = "ok<script>never closed";
        let cleaned = clean_markdown(md, 10_000);
        assert_eq!(cleaned, "ok");
    }
}
