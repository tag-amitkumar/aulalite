// crates/backend/tests/fixtures/mod.rs
//! Shared helpers for Phase 1a integration tests.
//!
//! Every test creates fresh tenants and users (UUID-suffixed) so tests can
//! run in parallel against the same Postgres without collisions.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:55432/aulalite".into());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap()
}

pub async fn create_tenant(pool: &PgPool) -> Uuid {
    let slug = format!("t-{}", Uuid::new_v4());
    // Integration fixtures intentionally model a pre-ownership legacy shell;
    // individual tests attach the roles they need. Production inserts use the
    // ownership default and must commit with an owner or pending owner invite.
    sqlx::query_scalar(
        "INSERT INTO tenants (slug, name, ownership_initialized_at)
         VALUES ($1, $1, NULL) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

pub async fn create_user(pool: &PgPool) -> (Uuid, String, String) {
    let firebase_uid = format!("fbuid-{}", Uuid::new_v4());
    let email = format!("u-{}@example.test", Uuid::new_v4());
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO users (firebase_uid, email) VALUES ($1, $2) RETURNING id")
            .bind(&firebase_uid)
            .bind(&email)
            .fetch_one(pool)
            .await
            .unwrap();
    (id, firebase_uid, email)
}

pub async fn attach_membership(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    role: &str, // 'org_owner' | 'org_admin' | 'teacher' | 'ta' | 'student' | 'parent'
) {
    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
         VALUES ($1, $2, $3, 'active')",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

#[derive(Clone)]
pub struct StubAuth {
    pub pool: PgPool,
    pub user_id: Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub tenant_id: Option<Uuid>,
    pub tenant_role: Option<core_types::TenantRole>,
}

pub async fn stub_middleware(
    axum::extract::State(state): axum::extract::State<StubAuth>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, backend::error::ApiError> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;
    if let Some(tid) = state.tenant_id {
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tid.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;
    }
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(state.user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;

    let is_platform_admin: bool =
        sqlx::query_scalar("SELECT is_platform_admin FROM users WHERE id = $1")
            .bind(state.user_id)
            .fetch_one(&state.pool)
            .await
            .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;

    req.extensions_mut()
        .insert(backend::context::RequestContext {
            user_id: state.user_id,
            firebase_uid: state.firebase_uid.clone(),
            email: state.email.clone(),
            display_name: None,
            tenant_id: state.tenant_id,
            tenant_role: state.tenant_role,
            is_platform_admin,
            identity_scope: backend::auth::identity::IdentityScope::Global,
        });

    Ok(next.run(req).await)
}

/// Build an axum Router that wraps `app_router` with stub-auth middleware.
pub fn build_test_app(app_router: Router, stub: StubAuth) -> Router {
    app_router.layer(middleware::from_fn_with_state(stub, stub_middleware))
}

/// Build a test app WITHOUT the StubAuth middleware. Used for routes that
/// are server-to-server (no Firebase ID token).
pub fn build_test_app_no_auth(router: Router) -> Router {
    router
}

/// Binds the given app to a free local port and returns the address. Useful
/// for tests that need a real HTTP/WebSocket server.
pub async fn bind_test_server(router: Router, auth: StubAuth) -> std::net::SocketAddr {
    let app = build_test_app(router, auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Fire an HTTP request against the test app, return (status, json body).
pub async fn fire(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer stubbed");
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let req = match body {
        Some(json) => builder.body(Body::from(json.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}
