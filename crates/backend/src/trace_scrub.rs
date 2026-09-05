//! Span scrubber that drops query strings from logged URIs.

use axum::http::{Request, Uri};
use tower_http::trace::MakeSpan;
use tracing::Span;

#[derive(Clone, Debug, Default)]
pub struct ScrubbingMakeSpan;

impl<B> MakeSpan<B> for ScrubbingMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let scrubbed = scrub_uri(request.uri());
        tracing::info_span!(
            "http_request",
            method = %request.method(),
            uri = %scrubbed,
            request_id = %request
                .headers()
                .get(crate::request_trace::REQUEST_ID_HEADER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("missing"),
        )
    }
}

pub fn scrub_uri(uri: &Uri) -> String {
    // Query values routinely contain bearer tokens, email addresses, search
    // text, and provider callback state. Keeping an allow/deny list is brittle;
    // the path plus request id is sufficient for operational correlation.
    uri.path().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_the_entire_query_string() {
        let uri = Uri::from_static("/v1/sessions/abc/join?access_token=secret&foo=1&jwt=AAA");
        assert_eq!(scrub_uri(&uri), "/v1/sessions/abc/join");
    }

    #[test]
    fn preserves_path_when_no_query() {
        let uri = Uri::from_static("/v1/me");
        assert_eq!(scrub_uri(&uri), "/v1/me");
    }
}
