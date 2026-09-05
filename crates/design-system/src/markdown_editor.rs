// crates/design-system/src/markdown_editor.rs
use dioxus::prelude::*;
use pulldown_cmark::{html, CowStr, Event, Parser, Tag};

/// Render untrusted markdown to HTML with raw-HTML passthrough neutralized.
///
/// This is the security-critical boundary for the multi-tenant LMS: markdown is
/// user-authored, so we must not let raw inline/block HTML reach the DOM.
///
/// pulldown-cmark emits any literal HTML in the source as `Event::Html` (block)
/// and `Event::InlineHtml` (inline) passthrough events, which its HTML renderer
/// would write verbatim. We map those events to `Event::Text` of the same
/// content; the renderer HTML-escapes `Event::Text`, so `<script>` becomes the
/// visible string `&lt;script&gt;` instead of executing markup.
///
/// Link and image targets use a strict allowlist: HTTP(S), mail links, and
/// local relative references. Everything else is replaced with an inert
/// fragment so uncommon or browser-dependent schemes cannot become an XSS
/// bypass.
pub fn render_markdown_safe(input: &str) -> String {
    let parser = Parser::new(input).map(neutralize_event);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn neutralize_event(event: Event<'_>) -> Event<'_> {
    match event {
        // Raw HTML passthrough -> escaped text so it renders inert.
        Event::Html(html) => Event::Text(html),
        Event::InlineHtml(html) => Event::Text(html),
        // Sanitize URL schemes on links and images.
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: sanitize_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: sanitize_url(dest_url),
            title,
            id,
        }),
        other => other,
    }
}

/// Keep only explicitly supported URL forms. A fragment is a safer fallback
/// than `about:blank`: it remains in-document and itself belongs to the
/// relative-reference allowlist.
fn sanitize_url(url: CowStr<'_>) -> CowStr<'_> {
    if is_allowed_url(&url) {
        url
    } else {
        CowStr::Borrowed("#")
    }
}

fn is_allowed_url(url: &str) -> bool {
    let trimmed = url
        .trim_matches(|character: char| character.is_ascii_whitespace() || character.is_control());
    if trimmed.is_empty() {
        return true;
    }

    // Network-path references can silently navigate away from the academy
    // despite having no scheme. Treat only local path/query/fragment forms as
    // relative. Backslashes are included because URL parsers normalize them
    // to slashes for special schemes in some browser contexts.
    if trimmed.starts_with("//")
        || trimmed.starts_with("\\\\")
        || trimmed.starts_with("/\\")
        || trimmed.starts_with("\\/")
    {
        return false;
    }

    let first_path_delimiter = trimmed.find(['/', '?', '#']).unwrap_or(trimmed.len());
    if let Some(colon) = trimmed.find(':') {
        if colon < first_path_delimiter {
            // Browsers ignore ASCII tabs/newlines in schemes. Remove all
            // whitespace/control characters before comparing so payloads such
            // as `java\nscript:` cannot evade the allowlist.
            let scheme: String = trimmed[..colon]
                .chars()
                .filter(|character| !character.is_ascii_whitespace() && !character.is_control())
                .flat_map(char::to_lowercase)
                .collect();
            return matches!(scheme.as_str(), "http" | "https" | "mailto");
        }
    }

    // A URL without a leading scheme delimiter is a local relative reference.
    true
}

#[derive(Props, Clone, PartialEq)]
pub struct MarkdownEditorProps {
    pub value: String,
    pub on_change: EventHandler<String>,
    #[props(default = false)]
    pub disabled: bool,
}

#[derive(Clone, PartialEq)]
enum Mode {
    Edit,
    Preview,
}

#[component]
pub fn MarkdownEditor(props: MarkdownEditorProps) -> Element {
    let mut mode = use_signal(|| Mode::Edit);
    let on_change = props.on_change;

    let preview_html = render_markdown_safe(&props.value);

    rsx! {
        div { class: "ds-md-editor",
            div { class: "ds-md-tabs",
                button {
                    class: if matches!(*mode.read(), Mode::Edit) { "ds-tab ds-tab--active" } else { "ds-tab" },
                    onclick: move |_| mode.set(Mode::Edit),
                    "Edit"
                }
                button {
                    class: if matches!(*mode.read(), Mode::Preview) { "ds-tab ds-tab--active" } else { "ds-tab" },
                    onclick: move |_| mode.set(Mode::Preview),
                    "Preview"
                }
            }
            match *mode.read() {
                Mode::Edit => rsx! {
                    textarea {
                        class: "ds-md-textarea",
                        value: "{props.value}",
                        disabled: props.disabled,
                        oninput: move |evt| on_change.call(evt.value()),
                    }
                },
                Mode::Preview => rsx! {
                    div { class: "ds-md-preview", dangerous_inner_html: "{preview_html}" }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_allowed_url, render_markdown_safe};

    #[test]
    fn escapes_raw_block_and_inline_html() {
        let input = "<script>alert(1)</script>\n\nText with <img src=x onerror=alert(1)> inline.";
        let out = render_markdown_safe(input);

        // No live tags: the dangerous markup must be escaped, not passed through.
        assert!(
            !out.contains("<script>"),
            "raw <script> tag leaked into output: {out}"
        );
        assert!(
            !out.contains("<img"),
            "raw <img> tag leaked into output: {out}"
        );
        // The `onerror=` substring is allowed only as inert escaped text inside
        // an `&lt;img ...&gt;` sequence — never as a live attribute on a real
        // `<img` element (which the assertion above already rules out).

        // The content is still present, but escaped to visible text.
        assert!(
            out.contains("&lt;script&gt;"),
            "script tag should be escaped to text: {out}"
        );
        assert!(
            out.contains("&lt;img"),
            "img tag should be escaped to text: {out}"
        );
    }

    #[test]
    fn renders_normal_markdown() {
        let out = render_markdown_safe("# Heading\n\n**bold** and [link](https://example.com)");
        assert!(out.contains("<h1>"), "heading not rendered: {out}");
        assert!(out.contains("Heading"), "heading text missing: {out}");
        assert!(
            out.contains("<strong>bold</strong>"),
            "bold not rendered: {out}"
        );
        assert!(
            out.contains("href=\"https://example.com\""),
            "link href not rendered: {out}"
        );
    }

    #[test]
    fn neutralizes_javascript_link_scheme() {
        let out = render_markdown_safe("[click](javascript:alert(1))");
        assert!(
            !out.contains("javascript:"),
            "javascript: scheme leaked into link href: {out}"
        );
        assert!(
            out.contains("href=\"#\""),
            "dangerous link should be replaced with an inert fragment: {out}"
        );
    }

    #[test]
    fn neutralizes_data_links_and_images() {
        let link = render_markdown_safe("[x](data:text/html,<script>alert(1)</script>)");
        assert!(
            link.contains("href=\"#\""),
            "data: link should be neutralized: {link}"
        );

        let img = render_markdown_safe("![alt](data:image/png;base64,iVBORw0KGgo=)");
        assert!(
            !img.contains("data:image"),
            "data: image URL should be neutralized: {img}"
        );
    }

    #[test]
    fn url_allowlist_rejects_obfuscated_and_unapproved_schemes() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "java\rscript:alert(1)",
            "  vbscript:msgbox(1)",
            "data:text/html,hello",
            "ftp://example.com/file",
            "//evil.example/path",
            "\\\\evil.example\\path",
        ] {
            assert!(!is_allowed_url(url), "dangerous URL was allowed: {url:?}");
        }
    }

    #[test]
    fn url_allowlist_accepts_supported_and_local_destinations() {
        for url in [
            "https://example.com/course",
            "HTTP://example.com",
            "mailto:teacher@example.com",
            "/courses/one",
            "courses/one",
            "./lesson",
            "../lesson",
            "#week-2",
            "?page=2",
        ] {
            assert!(is_allowed_url(url), "safe URL was rejected: {url:?}");
        }
    }
}
