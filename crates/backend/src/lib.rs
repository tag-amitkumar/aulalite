// crates/backend/src/lib.rs
#![allow(
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

pub mod auth;
pub mod context;
pub mod db;
pub mod error;
pub mod handlers;
pub mod request_trace;
pub mod services;
pub mod storage;
pub mod trace_scrub;

use std::sync::Arc;

use axum::http::{header, HeaderValue, Method};
use axum::{middleware, routing::get, Router};
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::local_login::LocalLoginConfig;
use crate::auth::middleware::{require_auth, AuthState};
use crate::auth::verify::Verifier;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub verifier: Arc<Verifier>,
    pub email_link_sender: Arc<dyn crate::services::invitations::EmailLinkSender>,
    /// Canonical SPA origin used for user-facing deep links and redirects.
    pub app_origin: String,
    /// Canonical public API origin used for SSO/LTI callbacks and API links.
    pub api_origin: String,
    /// Exact browser origins admitted by API and object-store CORS.
    pub browser_origins: Vec<String>,
    /// Environment-configured global identities granted platform authority.
    pub super_admins: crate::auth::super_admin::SuperAdminConfig,
    pub storage: Arc<dyn crate::storage::S3Client>,
    pub bucket_name: String,
    pub mediamtx: Arc<dyn crate::services::mediamtx::MediaMtxClient>,
    pub jwt_signer: Arc<crate::services::mediamtx::JwtSigner>,
    pub mediamtx_public_webrtc_url: String,
    pub mediamtx_public_hls_url: String,
    pub live_room: Arc<dyn crate::services::live_room::LiveRoomBroker>,
    pub recorder: Arc<dyn crate::services::recording::RecorderTool>,
    pub recordings_dir: String,
    pub local_login: LocalLoginConfig,
    /// Shared-secret required on the MediaMTX → backend auth callback.
    /// Set via `MEDIAMTX_AUTH_SHARED_HEADER` env. When `Some`, the
    /// `/v1/mediamtx/auth/publish` handler requires either an
    /// `X-MediaMTX-Auth-Shared` header or `?shared=` query param matching
    /// this value. When `None`, the check is skipped (dev/test mode).
    pub mediamtx_auth_shared_secret: Option<String>,
    /// Stripe billing client. Production uses `HttpStripeClient`; outside
    /// production (and in tests) a `MockStripeClient` is injected so billing
    /// compiles and runs without live Stripe keys.
    pub stripe: Arc<dyn crate::services::billing::StripeClient>,
    /// Stripe webhook signing secret (`whsec_…`). `Some` in production (hard
    /// error if unset); `None` outside production, where the webhook handler
    /// must treat verification as unconfigured.
    pub stripe_webhook_secret: Option<String>,
    /// Transactional email sender for notifications. Production uses
    /// `ResendEmailNotifier`; outside production (and in tests) a
    /// `MockEmailNotifier` is injected so notifications compile and run without
    /// a live email provider.
    pub email_notifier: Arc<dyn crate::services::notifications::EmailNotifier>,
    /// Push sender for notifications. Production uses `FcmPushSender` (currently
    /// scaffolding); outside production (and in tests) a `MockPushSender` is
    /// injected.
    pub push_sender: Arc<dyn crate::services::notifications::PushSender>,
    /// Per-client Redis sliding-window API rate limiter. `Some` only when
    /// `AULALITE_RATE_LIMIT=on`; `None` (default) installs no layer, so dev and
    /// tests are unaffected.
    pub rate_limit: Option<crate::services::rate_limit::RateLimitState>,
}

pub fn router(state: AppState) -> Router {
    let auth_state = AuthState {
        pool: state.pool.clone(),
        verifier: state.verifier.clone(),
        local_login: state.local_login.clone(),
        super_admins: state.super_admins.clone(),
    };

    let mut public = Router::new()
        .route("/healthz", get(handlers::health::healthz))
        // Unauthenticated Prometheus scrape endpoint (network-restricted at the
        // proxy/firewall, like /healthz). Emits only method/route-template/status
        // labels — no tenant data or secrets.
        .route("/metrics", get(crate::services::metrics::metrics_handler));

    // Dev-only routes — registered ONLY when their gating flag is true at
    // startup. The handlers also have runtime guards (defence-in-depth), but
    // not registering them keeps the routes invisible to scanners in prod.
    if state.local_login.is_enabled() {
        public = public.merge(handlers::dev_login::routes(
            handlers::dev_login::DevLoginState {
                pool: state.pool.clone(),
                config: state.local_login.clone(),
            },
        ));
    }
    let audit_seed_enabled = env_bool("LOCAL_AUDIT_SEED_ENABLED");
    if audit_seed_enabled && state.local_login.is_enabled() {
        public = public.merge(handlers::dev_seed::routes(
            handlers::dev_seed::DevSeedState {
                pool: state.pool.clone(),
                config: state.local_login.clone(),
                enabled: audit_seed_enabled,
            },
        ));
    }
    // Enterprise SSO (OIDC): unauthenticated /v1/sso/:slug/start + /callback.
    // Merged into the stateless `public` router because public_routes bakes in
    // its own SsoState (Router<()>), exactly like dev_login::routes.
    public = public.merge(handlers::sso::public_routes(handlers::sso::SsoState {
        pool: state.pool.clone(),
        app_origin: state.app_origin.clone(),
        api_origin: state.api_origin.clone(),
        session_secret: crate::services::oidc::session_secret_from_env(),
    }));
    let authed = Router::new()
        .route("/v1/me", get(handlers::me::me))
        .route(
            "/v1/me/workspaces",
            axum::routing::get(handlers::me::my_workspaces),
        )
        .route(
            "/v1/me/courses",
            axum::routing::get(handlers::me::my_courses),
        )
        .route(
            "/v1/me/schedule",
            axum::routing::get(handlers::me::my_schedule),
        )
        .route(
            "/v1/me/active-session",
            axum::routing::get(handlers::me::my_active_session),
        )
        .route(
            "/v1/me/preferences",
            axum::routing::get(handlers::me::my_preferences)
                .patch(handlers::me::patch_my_preferences),
        )
        .route(
            "/v1/me/transcript",
            axum::routing::get(handlers::me::my_transcript),
        )
        .merge(handlers::courses::routes())
        .merge(handlers::modules::routes())
        .merge(handlers::lessons::routes())
        .merge(handlers::progress::routes())
        .merge(handlers::quizzes::routes())
        .merge(handlers::gamification::routes())
        .merge(handlers::certificates::routes())
        .merge(handlers::flashcards::routes())
        .merge(handlers::assignments::routes())
        .merge(handlers::submissions::routes())
        .merge(handlers::enrollments::routes())
        .merge(handlers::enrollments::invitation_routes())
        .merge(handlers::live_sessions::routes())
        .merge(handlers::live_sessions::live_room_routes())
        .merge(handlers::uploads::routes())
        .merge(handlers::file_assets::routes())
        .merge(handlers::admin::routes())
        .merge(handlers::billing::routes())
        .merge(handlers::parent::routes())
        .merge(handlers::platform::routes())
        .merge(handlers::member_invitations::routes())
        .merge(handlers::analytics::routes())
        .merge(handlers::notifications::routes())
        .merge(handlers::search::routes())
        .merge(handlers::announcements::routes())
        .merge(handlers::attendance_export::routes())
        .merge(handlers::gradebook::routes())
        .merge(handlers::rubrics::routes())
        .merge(handlers::bulk::routes())
        .merge(handlers::recordings::routes())
        .merge(handlers::session_feedback::routes())
        .merge(handlers::discussions::routes())
        .merge(handlers::calendar::routes())
        .merge(handlers::notes::routes())
        .merge(handlers::privacy::routes())
        .merge(handlers::peer_review::routes())
        .merge(handlers::plagiarism::routes())
        .merge(handlers::public_api::admin_routes())
        .merge(handlers::webhooks::routes())
        .merge(handlers::scorm::routes())
        .merge(handlers::lti::admin_routes())
        .merge(handlers::mfa::routes())
        .merge(handlers::sso::admin_routes())
        .layer(middleware::from_fn_with_state(auth_state, require_auth))
        .with_state(state.clone());

    let mediamtx_callbacks = Router::new()
        .route("/readyz", axum::routing::get(handlers::health::readyz))
        .route(
            "/v1/mediamtx/auth/publish",
            axum::routing::post(handlers::live_sessions::mediamtx_auth_publish_pub),
        )
        .route(
            "/v1/mediamtx/jwks",
            axum::routing::get(handlers::live_sessions::mediamtx_jwks),
        )
        // Stripe webhook: PUBLIC, no Firebase token. Merged here alongside the
        // mediamtx callbacks (also outside the require_auth layer). The handler
        // authenticates via the Stripe signature header instead.
        .merge(handlers::billing::webhook_routes())
        // Certificate verification: PUBLIC by design — anyone with a
        // credential id can check validity (returns only public fields via a
        // SECURITY DEFINER lookup).
        .merge(handlers::certificates::public_routes())
        // Read-only public API authenticated by tenant-scoped API keys
        // (Authorization: Bearer ak_...). PUBLIC like the other callbacks here:
        // no Firebase token / require_auth layer — each handler authenticates
        // the api-key header itself and scopes reads to the key's tenant.
        .merge(handlers::public_api::routes())
        // LTI 1.3 (Tool side): unauthenticated OIDC third-party login init +
        // launch (each validates the platform id_token against its JWKS).
        .merge(handlers::lti::public_routes())
        .with_state(state.clone());

    // CORS: lock to the explicit application + admin origins. The API is on a
    // separate hostname, so both browser frontends require cross-origin access.
    let cors = cors_layer(&state.browser_origins);

    // Security headers (set on every response). HSTS is only meaningful over
    // HTTPS; behind a TLS-terminating proxy the browser still sees these.
    let security_headers = tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        // CSP intentionally minimal — the API never serves HTML. If a future
        // change starts returning HTML, tighten this with a per-route layer.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ));

    let mut app = Router::new()
        .merge(public)
        .merge(authed)
        .merge(mediamtx_callbacks)
        .layer(cors)
        .layer(security_headers);

    // Opt-in global API rate limiter (AULALITE_RATE_LIMIT=on). Applied here so
    // it wraps every route but stays INSIDE the trace layer below, so throttled
    // (429) requests are still logged. `None` (the default) installs nothing.
    if let Some(rl) = state.rate_limit.clone() {
        app = app.layer(axum::middleware::from_fn_with_state(
            rl,
            crate::services::rate_limit::rate_limit,
        ));
    }

    // Metrics timing layer: records per-route request count + duration into the
    // process-global registry exposed at GET /metrics. Installed on the merged
    // router so axum's MatchedPath (route template) is available to the
    // middleware. Inside the trace layer (below) so spans still cover it, and
    // after the rate-limit layer so throttled 429s are still counted.
    app = app.layer(axum::middleware::from_fn(
        crate::services::metrics::track_metrics,
    ));

    // Request correlation is outermost so the inner trace span can record the
    // normalized id. The same id is returned to clients on every response.
    app.layer(
        tower_http::trace::TraceLayer::new_for_http()
            .make_span_with(crate::trace_scrub::ScrubbingMakeSpan)
            // tower-http defaults successful responses to DEBUG, which makes
            // correlation ids disappear under the production INFO filter.
            .on_response(tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO)),
    )
    .layer(middleware::from_fn(crate::request_trace::correlate_request))
}

pub fn router_for_tests() -> Router {
    Router::new().route("/healthz", get(handlers::health::healthz))
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn parse_browser_origins(raw: &str) -> Result<Vec<String>, String> {
    let mut origins = Vec::new();
    for candidate in raw.split(',') {
        let candidate = candidate.trim().trim_end_matches('/');
        if candidate.is_empty() {
            continue;
        }
        let parsed = reqwest::Url::parse(candidate)
            .map_err(|error| format!("invalid browser origin `{candidate}`: {error}"))?;
        let canonical = matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none();
        if !canonical {
            return Err(format!(
                "browser origin must be a bare HTTP(S) origin: {candidate}"
            ));
        }
        if !origins.iter().any(|origin| origin == candidate) {
            origins.push(candidate.to_string());
        }
    }
    if origins.is_empty() {
        return Err("at least one browser origin is required".into());
    }
    Ok(origins)
}

fn cors_layer(browser_origins: &[String]) -> CorsLayer {
    let origins: Option<Vec<HeaderValue>> = browser_origins
        .iter()
        .map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    match origins.filter(|origins| !origins.is_empty()) {
        Some(origins) => CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PATCH,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                header::ACCEPT,
                header::HeaderName::from_static(crate::auth::middleware::TENANT_HEADER),
                crate::request_trace::request_id_header(),
            ])
            .expose_headers([crate::request_trace::request_id_header()])
            .allow_credentials(true),
        None => {
            tracing::warn!(
                origins = ?browser_origins,
                "AULALITE_BROWSER_ORIGINS contains an invalid header value; CORS is closed"
            );
            CorsLayer::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cors_layer, parse_browser_origins};
    use axum::body::Body;
    use axum::http::{header, Method, Request};
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn invalid_cors_origin_fails_closed() {
        let app = Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(cors_layer(&[
                "https://good.example\r\nhttps://evil.example".into(),
            ]));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/probe")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            "invalid APP_ORIGIN must not fall back to permissive CORS"
        );
    }

    #[test]
    fn browser_origin_config_is_canonical_and_deduplicated() {
        let origins = parse_browser_origins(
            "https://aula.elementors.guru, https://admin.elementors.guru/,https://aula.elementors.guru",
        )
        .unwrap();
        assert_eq!(
            origins,
            vec![
                "https://aula.elementors.guru",
                "https://admin.elementors.guru"
            ]
        );
        assert!(parse_browser_origins("https://example.com/path").is_err());
        assert!(parse_browser_origins("").is_err());
    }
}
