// crates/features-courses/src/markdown.rs
//! Shared read-only markdown rendering for user-authored course content.

pub(crate) fn render_user_markdown_html(input: &str) -> String {
    design_system::render_markdown_safe(input)
}

#[cfg(test)]
mod tests {
    use super::render_user_markdown_html;

    #[test]
    fn read_only_markdown_escapes_raw_html() {
        let html = render_user_markdown_html("<script>alert(1)</script>");

        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn read_only_markdown_rewrites_dangerous_links() {
        let html = render_user_markdown_html("[bad](javascript:alert(1))");

        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("href=\"#\""), "{html}");
    }
}
