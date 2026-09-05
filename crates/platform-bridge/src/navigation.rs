//! Renderer-neutral policy for deciding which links belong to the in-app
//! router. Native WebViews use this policy through a click bridge; browser
//! builds retain their normal anchor behavior.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDisposition {
    InternalRoute(String),
    BrowserDefault,
}

/// Classify one link activation without changing normal browser semantics for
/// modified clicks, downloads, new-window targets, fragments, or external URLs.
pub fn classify_link_activation(
    href: &str,
    target: Option<&str>,
    has_download: bool,
    mouse_button: u16,
    has_modifier: bool,
) -> LinkDisposition {
    if mouse_button != 0
        || has_modifier
        || has_download
        || target.is_some_and(|value| !value.is_empty() && !value.eq_ignore_ascii_case("_self"))
    {
        return LinkDisposition::BrowserDefault;
    }

    match internal_route_path(href) {
        Some(path) => LinkDisposition::InternalRoute(path),
        None => LinkDisposition::BrowserDefault,
    }
}

/// Return a normalized same-app path suitable for programmatic router
/// navigation. Scheme-relative URLs (`//host/path`) and control characters are
/// rejected even though they start with a slash.
pub fn internal_route_path(href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with('/') && !href.starts_with("//") && !href.chars().any(char::is_control) {
        Some(href.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_same_app_links_use_the_native_router() {
        assert_eq!(
            classify_link_activation(
                "/courses/physics?tab=assignments#latest",
                None,
                false,
                0,
                false,
            ),
            LinkDisposition::InternalRoute("/courses/physics?tab=assignments#latest".to_string())
        );
        assert_eq!(
            internal_route_path(" /admin/billing "),
            Some("/admin/billing".into())
        );
    }

    #[test]
    fn external_special_and_modified_links_keep_platform_defaults() {
        for disposition in [
            classify_link_activation("https://elementors.guru", None, false, 0, false),
            classify_link_activation("mailto:hello@elementors.guru", None, false, 0, false),
            classify_link_activation("#main", None, false, 0, false),
            classify_link_activation("//cdn.example.test/file", None, false, 0, false),
            classify_link_activation("/report.csv", Some("_blank"), false, 0, false),
            classify_link_activation("/report.csv", None, true, 0, false),
            classify_link_activation("/courses", None, false, 1, false),
            classify_link_activation("/courses", None, false, 0, true),
        ] {
            assert_eq!(disposition, LinkDisposition::BrowserDefault);
        }
    }

    #[test]
    fn programmatic_navigation_rejects_external_and_unsafe_targets() {
        assert_eq!(
            internal_route_path("/search?q=calculus"),
            Some("/search?q=calculus".into())
        );
        assert_eq!(internal_route_path("https://evil.example"), None);
        assert_eq!(internal_route_path("//evil.example"), None);
        assert_eq!(internal_route_path("/courses\nSet-Cookie: x"), None);
    }
}
