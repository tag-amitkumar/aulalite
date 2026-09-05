//! Integration test for /v1/me with stubbed claims instead of Firebase.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:55432/aulalite".into());
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap()
}

#[derive(Clone)]
struct StubAuth {
    pool: sqlx::PgPool,
    firebase_uid: String,
    email: String,
}

async fn stub_middleware(
    axum::extract::State(state): axum::extract::State<StubAuth>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, backend::error::ApiError> {
    let claims = backend::auth::verify::FirebaseClaims {
        sub: state.firebase_uid.clone(),
        email: Some(state.email.clone()),
        email_verified: Some(true),
        name: None,
        picture: None,
        aud: "aulalite-dev".into(),
        iss: "https://securetoken.google.com/aulalite-dev".into(),
        exp: chrono::Utc::now().timestamp() + 600,
        iat: chrono::Utc::now().timestamp(),
        auth_time: None,
    };
    let user = backend::auth::jit_provision::ensure_user(&state.pool, &claims)
        .await
        .map_err(|err| backend::error::ApiError::Internal(err.to_string()))?;
    let is_platform_admin: bool =
        sqlx::query_scalar("SELECT is_platform_admin FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&state.pool)
            .await
            .map_err(|err| backend::error::ApiError::Internal(err.to_string()))?;

    req.extensions_mut()
        .insert(backend::context::RequestContext {
            user_id: user.user_id,
            firebase_uid: user.firebase_uid,
            email: user.email,
            display_name: user.display_name,
            tenant_id: None,
            tenant_role: None,
            is_platform_admin,
            identity_scope: backend::auth::identity::IdentityScope::Global,
        });

    Ok(next.run(req).await)
}

#[tokio::test]
async fn me_returns_user_after_jit_provision() {
    let pool = pool().await;
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4());
    let email = format!("u_{}@example.test", Uuid::new_v4());

    let stub = StubAuth {
        pool: pool.clone(),
        firebase_uid: firebase_uid.clone(),
        email: email.clone(),
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(stub, stub_middleware));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("authorization", "Bearer ignored-by-stub")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["firebase_uid"].as_str().unwrap(), firebase_uid);
    assert_eq!(
        json["email"].as_str().unwrap().to_lowercase(),
        email.to_lowercase()
    );
    assert_eq!(json["tenant_id"], serde_json::Value::Null);
}

#[tokio::test]
async fn user_preference_columns_default_update_and_reject_invalid() {
    let pool = pool().await;
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4());
    let email = format!("u_{}@example.test", Uuid::new_v4());
    let claims = backend::auth::verify::FirebaseClaims {
        sub: firebase_uid,
        email: Some(email),
        email_verified: Some(true),
        name: None,
        picture: None,
        aud: "aulalite-dev".into(),
        iss: "https://securetoken.google.com/aulalite-dev".into(),
        exp: chrono::Utc::now().timestamp() + 600,
        iat: chrono::Utc::now().timestamp(),
        auth_time: None,
    };
    let user = backend::auth::jit_provision::ensure_user(&pool, &claims)
        .await
        .unwrap();

    // New users default to system/comfortable (migration 20260610000031).
    let (theme, density): (String, String) =
        sqlx::query_as("SELECT theme_preference, density_preference FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (theme.as_str(), density.as_str()),
        ("system", "comfortable")
    );

    // The PATCH handler's COALESCE update path persists valid values.
    let (theme, density): (String, String) = sqlx::query_as(
        "UPDATE users SET theme_preference = COALESCE($2, theme_preference), \
         density_preference = COALESCE($3, density_preference) \
         WHERE id = $1 RETURNING theme_preference, density_preference",
    )
    .bind(user.user_id)
    .bind(Some("dark"))
    .bind(None::<&str>)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((theme.as_str(), density.as_str()), ("dark", "comfortable"));

    // CHECK constraints reject unknown values defense-in-depth (the handler
    // validates first; the schema backstops direct writes).
    let err = sqlx::query("UPDATE users SET theme_preference = 'midnight' WHERE id = $1")
        .bind(user.user_id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("users_theme_preference_check"),
        "expected check-constraint violation, got: {err}"
    );
}
