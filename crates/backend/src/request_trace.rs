//! Request correlation shared by HTTP responses and structured tracing spans.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn request_id_header() -> HeaderName {
    HeaderName::from_static(REQUEST_ID_HEADER)
}

fn valid_request_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn normalized_request_id(request: &Request) -> HeaderValue {
    request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_request_id(value))
        .and_then(|value| HeaderValue::from_str(value).ok())
        .unwrap_or_else(|| HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap())
}

/// Preserve a safe upstream correlation id or generate one, then make it
/// available to the inner trace layer and return it on every response.
pub async fn correlate_request(mut request: Request, next: Next) -> Response {
    let request_id = normalized_request_id(&request);
    request
        .headers_mut()
        .insert(request_id_header(), request_id.clone());

    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(request_id_header(), request_id);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::{middleware, routing::get, Router};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(middleware::from_fn(correlate_request))
    }

    #[tokio::test]
    async fn preserves_safe_upstream_request_id() {
        let response = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/probe")
                    .header(REQUEST_ID_HEADER, "edge-01HZY7X9A2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.headers().get(REQUEST_ID_HEADER).unwrap(),
            "edge-01HZY7X9A2"
        );
    }

    #[tokio::test]
    async fn replaces_unsafe_request_id() {
        let response = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/probe")
                    .header(REQUEST_ID_HEADER, "attacker supplied spaces")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let value = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(Uuid::parse_str(value).is_ok());
    }
}
