use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

mod fixtures;

use backend::auth::jwks::JwksCache;
use backend::auth::local_login::{LocalLoginConfig, LocalLoginProfile};
use backend::auth::middleware::{require_auth, AuthState};
use backend::auth::verify::Verifier;
use backend::handlers::dev_login::{routes as dev_login_routes, DevLoginState};

fn local_config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env,
        enabled,
        vec![
            LocalLoginProfile {
                name: "teacher".to_string(),
                token: "local-teacher-token".to_string(),
                password: "teacher-pass".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            LocalLoginProfile {
                name: "student".to_string(),
                token: "local-student-token".to_string(),
                password: "student-pass".to_string(),
                email: "local.student@example.test".to_string(),
                display_name: "Local Student".to_string(),
                firebase_uid: "local-login-local-student".to_string(),
            },
        ],
    )
}

async fn json_response(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn post_local_login(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/auth/local-login")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn local_login_email_password_grants_teacher_token() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-teacher-token");
}

#[tokio::test]
async fn local_login_email_password_grants_student_token() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.student@example.test","password":"student-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-student-token");
}

#[tokio::test]
async fn local_login_case_insensitive_email_matches() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"LOCAL.TEACHER@EXAMPLE.TEST","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-teacher-token");
}

#[tokio::test]
async fn local_login_wrong_password_returns_401() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"nope"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_text(response).await;
    assert!(
        !body.contains("local-teacher-token"),
        "401 body must not leak the token: {body}"
    );
}

#[tokio::test]
async fn local_login_unknown_email_returns_401() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"who@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn local_login_missing_password_field_returns_400() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn local_login_when_disabled_returns_403() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("production", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn local_token_authenticates_me_through_real_middleware() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
        super_admins: backend::auth::super_admin::SuperAdminConfig::default(),
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("authorization", "Bearer local-teacher-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["firebase_uid"], "local-login-local-teacher");
    assert_eq!(json["email"], "local.teacher@example.test");
    assert_eq!(json["display_name"], "Local Teacher");
}

#[tokio::test]
async fn access_token_query_rejected_on_non_upgrade() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
        super_admins: backend::auth::super_admin::SuperAdminConfig::default(),
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn access_token_query_accepted_when_ws_upgrade() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
        super_admins: backend::auth::super_admin::SuperAdminConfig::default(),
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn access_token_url_decoded_on_ws_upgrade() {
    let pool = fixtures::pool().await;
    let token = "tkn+/abc=";
    let encoded = "tkn%2B%2Fabc%3D";
    let config = LocalLoginConfig::from_profiles(
        "local",
        true,
        vec![LocalLoginProfile {
            name: "teacher".to_string(),
            token: token.to_string(),
            password: "teacher-pass".to_string(),
            email: "local.teacher@example.test".to_string(),
            display_name: "Local Teacher".to_string(),
            firebase_uid: "local-login-local-teacher".to_string(),
        }],
    );
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
        super_admins: backend::auth::super_admin::SuperAdminConfig::default(),
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/me?access_token={encoded}"))
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
