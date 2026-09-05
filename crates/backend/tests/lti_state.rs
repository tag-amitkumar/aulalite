//! LTI login-state persistence tests.
//!
//! These require the same live Postgres test database as the other backend
//! integration tests. They cover the shared store that backs the unauthenticated
//! /v1/lti/login -> /v1/lti/launch state/nonce round-trip.

mod fixtures;

use fixtures::pool;
use uuid::Uuid;

#[tokio::test]
async fn lti_login_state_is_single_use_and_preserves_target_link_uri() {
    let pool = pool().await;
    let state = format!("state-{}", Uuid::new_v4());

    backend::db::lti::insert_login_state(
        &pool,
        &state,
        "nonce-1",
        Some("https://app.example.com/courses/bio"),
    )
    .await
    .unwrap();

    let first = backend::db::lti::take_login_state(&pool, &state)
        .await
        .unwrap()
        .expect("state should be present");
    assert_eq!(first.nonce, "nonce-1");
    assert_eq!(
        first.target_link_uri.as_deref(),
        Some("https://app.example.com/courses/bio")
    );

    let second = backend::db::lti::take_login_state(&pool, &state)
        .await
        .unwrap();
    assert!(second.is_none(), "state must be single-use");
}

#[tokio::test]
async fn expired_lti_login_state_is_not_returned() {
    let pool = pool().await;
    let state = format!("state-{}", Uuid::new_v4());

    let mut tx = backend::db::begin_system_context(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO lti_login_states (state, nonce, target_link_uri, created_at)
         VALUES ($1, $2, $3, now() - interval '16 minutes')",
    )
    .bind(&state)
    .bind("nonce-expired")
    .bind(Option::<&str>::None)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let row = backend::db::lti::take_login_state(&pool, &state)
        .await
        .unwrap();
    assert!(row.is_none(), "expired state should not be returned");
}
