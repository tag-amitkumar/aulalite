//! Flashcard endpoints (learning-suite Cycle 6).
//!
//! Teacher (course staff):
//! - `GET    /v1/courses/{cid}/decks`                      — all decks (drafts included)
//! - `POST   /v1/courses/{cid}/decks`                      — create deck
//! - `PATCH  /v1/courses/{cid}/decks/{deck_id}`             — title/description/status
//! - `DELETE /v1/courses/{cid}/decks/{deck_id}`
//! - `GET    /v1/courses/{cid}/decks/{deck_id}/cards`       — full card list
//! - `POST   /v1/courses/{cid}/decks/{deck_id}/cards`
//! - `PATCH  /v1/courses/{cid}/decks/{deck_id}/cards/{card_id}`
//! - `DELETE /v1/courses/{cid}/decks/{deck_id}/cards/{card_id}`
//!
//! Student (course member; published decks only):
//! - `GET  /v1/courses/{cid}/decks` (published subset via the same route)
//! - `GET  /v1/courses/{cid}/decks/{deck_id}/review`        — due + new cards
//! - `POST /v1/courses/{cid}/decks/{deck_id}/review/{card_id}` — record a rating
//! - `GET  /v1/courses/{cid}/flashcards/due-count`         — badge count
//!
//! SM-2 scheduling is server-authoritative (`db::flashcards::next_review`,
//! mirroring the kinetics helper).

use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/decks",
            routing::get(list_decks).post(create_deck),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}",
            routing::patch(patch_deck).delete(delete_deck),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/cards",
            routing::get(list_cards).post(create_card),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/cards/{card_id}",
            routing::patch(patch_card).delete(delete_card),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/review",
            routing::get(review_queue),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/review/{card_id}",
            routing::post(record_review),
        )
        .route(
            "/v1/courses/{cid}/flashcards/due-count",
            routing::get(due_count),
        )
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/decks",
            routing::get(list_decks_t).post(create_deck_t),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}",
            routing::patch(patch_deck_t).delete(delete_deck_t),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/cards",
            routing::get(list_cards_t).post(create_card_t),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/cards/{card_id}",
            routing::patch(patch_card_t).delete(delete_card_t),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/review",
            routing::get(review_queue_t),
        )
        .route(
            "/v1/courses/{cid}/decks/{deck_id}/review/{card_id}",
            routing::post(record_review_t),
        )
        .route(
            "/v1/courses/{cid}/flashcards/due-count",
            routing::get(due_count_t),
        )
        .with_state(TestState { pool })
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn require_tenant(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no active tenant".into()))
}

async fn is_staff(pool: &PgPool, ctx: &RequestContext, course_id: Uuid) -> Result<bool, ApiError> {
    if !ctx.can_assist() {
        return Ok(false);
    }
    db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(internal)
}

async fn require_author(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !ctx.can_teach() || !is_staff(pool, ctx, course_id).await? {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn require_member(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    let ok = db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(internal)?;
    if ok {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Resolve a deck and confirm it belongs to `course_id` (404 otherwise).
async fn deck_in_course(
    conn: &mut sqlx::PgConnection,
    course_id: Uuid,
    deck_id: Uuid,
) -> Result<db::flashcards::DeckRow, ApiError> {
    let deck = db::flashcards::fetch_deck(conn, deck_id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    if deck.course_id != course_id {
        return Err(ApiError::NotFound);
    }
    Ok(deck)
}

// --- DTOs ---

#[derive(serde::Serialize)]
pub struct DeckDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub card_count: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::flashcards::DeckRow> for DeckDto {
    fn from(r: db::flashcards::DeckRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            title: r.title,
            description: r.description,
            status: r.status,
            card_count: r.card_count,
            created_at: r.created_at,
        }
    }
}

#[derive(serde::Deserialize)]
pub struct CreateDeckBody {
    pub title: String,
    pub description: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PatchDeckBody {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(serde::Serialize)]
pub struct CardDto {
    pub id: Uuid,
    pub deck_id: Uuid,
    pub position: i32,
    pub front: String,
    pub back: String,
}

#[derive(serde::Deserialize)]
pub struct CardBody {
    pub front: Option<String>,
    pub back: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ReviewCardDto {
    pub id: Uuid,
    pub front: String,
    pub back: String,
    /// None for never-reviewed (new) cards.
    pub repetitions: Option<i32>,
}

#[derive(serde::Deserialize)]
pub struct RatingBody {
    pub rating: String,
}

#[derive(serde::Serialize)]
pub struct ReviewOutcomeDto {
    pub card_id: Uuid,
    pub ease: f32,
    pub interval_days: f32,
    pub repetitions: i32,
}

// --- inner logic ---

async fn list_decks_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<DeckDto>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_member(pool, ctx, course_id).await?;
    let include_drafts = is_staff(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let rows = db::flashcards::list_decks(&mut tx, course_id, include_drafts)
        .await
        .map_err(internal)?;
    Ok(Json(rows.into_iter().map(DeckDto::from).collect()))
}

async fn create_deck_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    body: CreateDeckBody,
) -> Result<Json<DeckDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let id = db::flashcards::create_deck(
        &mut tx,
        tenant_id,
        course_id,
        title,
        body.description.as_deref(),
        ctx.user_id,
    )
    .await
    .map_err(internal)?;
    let deck = db::flashcards::fetch_deck(&mut tx, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::Internal("deck vanished".into()))?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(DeckDto::from(deck)))
}

async fn patch_deck_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
    body: PatchDeckBody,
) -> Result<Json<DeckDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    if let Some(status) = body.status.as_deref() {
        if !["draft", "published"].contains(&status) {
            return Err(ApiError::BadRequest("invalid status".into()));
        }
    }
    if let Some(title) = body.title.as_deref() {
        if title.trim().is_empty() {
            return Err(ApiError::BadRequest("title must not be empty".into()));
        }
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    db::flashcards::patch_deck(
        &mut tx,
        deck_id,
        body.title.as_deref().map(str::trim),
        body.description.as_deref(),
        body.status.as_deref(),
    )
    .await
    .map_err(internal)?;
    let deck = db::flashcards::fetch_deck(&mut tx, deck_id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(DeckDto::from(deck)))
}

async fn delete_deck_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    db::flashcards::delete_deck(&mut tx, deck_id)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn list_cards_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
) -> Result<Json<Vec<CardDto>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    if !is_staff(pool, ctx, course_id).await? {
        return Err(ApiError::Forbidden);
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    let cards = db::flashcards::list_cards(&mut tx, deck_id)
        .await
        .map_err(internal)?;
    Ok(Json(
        cards
            .into_iter()
            .map(|c| CardDto {
                id: c.id,
                deck_id: c.deck_id,
                position: c.position,
                front: c.front,
                back: c.back,
            })
            .collect(),
    ))
}

async fn create_card_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
    body: CardBody,
) -> Result<Json<CardDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    let front = body.front.as_deref().map(str::trim).unwrap_or_default();
    let back = body.back.as_deref().map(str::trim).unwrap_or_default();
    if front.is_empty() || back.is_empty() {
        return Err(ApiError::BadRequest("front and back are required".into()));
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    let id = db::flashcards::create_card(&mut tx, tenant_id, course_id, deck_id, front, back)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let cards = db::flashcards::list_cards(&mut tx, deck_id)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let card = cards
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| ApiError::Internal("card vanished".into()))?;
    Ok(Json(CardDto {
        id: card.id,
        deck_id: card.deck_id,
        position: card.position,
        front: card.front,
        back: card.back,
    }))
}

async fn patch_card_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
    card_id: Uuid,
    body: CardBody,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    let ok = db::flashcards::patch_card(
        &mut tx,
        tenant_id,
        course_id,
        deck_id,
        card_id,
        body.front.as_deref().map(str::trim),
        body.back.as_deref().map(str::trim),
    )
    .await
    .map_err(internal)?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    tx.commit().await.map_err(internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn delete_card_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
    card_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_author(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    deck_in_course(&mut tx, course_id, deck_id).await?;
    let ok = db::flashcards::delete_card(&mut tx, tenant_id, course_id, deck_id, card_id)
        .await
        .map_err(internal)?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    tx.commit().await.map_err(internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn review_queue_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
) -> Result<Json<Vec<ReviewCardDto>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_member(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let deck = deck_in_course(&mut tx, course_id, deck_id).await?;
    // Students review published decks only; staff may preview drafts.
    if deck.status != "published" && !is_staff(pool, ctx, course_id).await? {
        return Err(ApiError::NotFound);
    }
    let cards = db::flashcards::due_cards(&mut tx, deck_id, ctx.user_id)
        .await
        .map_err(internal)?;
    Ok(Json(
        cards
            .into_iter()
            .map(|c| ReviewCardDto {
                id: c.id,
                front: c.front,
                back: c.back,
                repetitions: c.repetitions,
            })
            .collect(),
    ))
}

async fn record_review_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    deck_id: Uuid,
    card_id: Uuid,
    body: RatingBody,
) -> Result<Json<ReviewOutcomeDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_member(pool, ctx, course_id).await?;
    let rating = body.rating.as_str();
    if !["again", "hard", "good", "easy"].contains(&rating) {
        return Err(ApiError::BadRequest(
            "rating must be one of again|hard|good|easy".into(),
        ));
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let deck = deck_in_course(&mut tx, course_id, deck_id).await?;
    if deck.status != "published" && !is_staff(pool, ctx, course_id).await? {
        return Err(ApiError::NotFound);
    }
    // The card must belong to the deck.
    let belongs: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1
               FROM flashcards f
               JOIN flashcard_decks d ON d.id = f.deck_id
              WHERE f.tenant_id = $1 AND f.id = $4 AND f.deck_id = $3
                AND d.tenant_id = $1 AND d.course_id = $2 AND d.id = $3
         )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(deck_id)
    .bind(card_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if !belongs {
        return Err(ApiError::NotFound);
    }
    let next = db::flashcards::record_review(&mut tx, tenant_id, card_id, ctx.user_id, rating)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(ReviewOutcomeDto {
        card_id,
        ease: next.ease,
        interval_days: next.interval_days,
        repetitions: next.repetitions,
    }))
}

async fn due_count_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_member(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let due = db::flashcards::due_count_for_course(&mut tx, course_id, ctx.user_id)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "due": due })))
}

// The production and test wrappers stay explicit because several routes have
// multi-segment paths or bodies, and the repeated signatures are easier to audit
// than a broad routing macro.

async fn list_decks(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<DeckDto>>, ApiError> {
    list_decks_inner(&s.pool, &ctx, cid).await
}
async fn list_decks_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<DeckDto>>, ApiError> {
    list_decks_inner(&s.pool, &ctx, cid).await
}

async fn create_deck(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateDeckBody>,
) -> Result<Json<DeckDto>, ApiError> {
    create_deck_inner(&s.pool, &ctx, cid, b).await
}
async fn create_deck_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateDeckBody>,
) -> Result<Json<DeckDto>, ApiError> {
    create_deck_inner(&s.pool, &ctx, cid, b).await
}

async fn patch_deck(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
    Json(b): Json<PatchDeckBody>,
) -> Result<Json<DeckDto>, ApiError> {
    patch_deck_inner(&s.pool, &ctx, cid, deck_id, b).await
}
async fn patch_deck_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
    Json(b): Json<PatchDeckBody>,
) -> Result<Json<DeckDto>, ApiError> {
    patch_deck_inner(&s.pool, &ctx, cid, deck_id, b).await
}

async fn delete_deck(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_deck_inner(&s.pool, &ctx, cid, deck_id).await
}
async fn delete_deck_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_deck_inner(&s.pool, &ctx, cid, deck_id).await
}

async fn list_cards(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<CardDto>>, ApiError> {
    list_cards_inner(&s.pool, &ctx, cid, deck_id).await
}
async fn list_cards_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<CardDto>>, ApiError> {
    list_cards_inner(&s.pool, &ctx, cid, deck_id).await
}

async fn create_card(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
    Json(b): Json<CardBody>,
) -> Result<Json<CardDto>, ApiError> {
    create_card_inner(&s.pool, &ctx, cid, deck_id, b).await
}
async fn create_card_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
    Json(b): Json<CardBody>,
) -> Result<Json<CardDto>, ApiError> {
    create_card_inner(&s.pool, &ctx, cid, deck_id, b).await
}

async fn patch_card(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<CardBody>,
) -> Result<axum::http::StatusCode, ApiError> {
    patch_card_inner(&s.pool, &ctx, cid, deck_id, card_id, b).await
}
async fn patch_card_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<CardBody>,
) -> Result<axum::http::StatusCode, ApiError> {
    patch_card_inner(&s.pool, &ctx, cid, deck_id, card_id, b).await
}

async fn delete_card(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_card_inner(&s.pool, &ctx, cid, deck_id, card_id).await
}
async fn delete_card_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_card_inner(&s.pool, &ctx, cid, deck_id, card_id).await
}

async fn review_queue(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ReviewCardDto>>, ApiError> {
    review_queue_inner(&s.pool, &ctx, cid, deck_id).await
}
async fn review_queue_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ReviewCardDto>>, ApiError> {
    review_queue_inner(&s.pool, &ctx, cid, deck_id).await
}

async fn record_review(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<RatingBody>,
) -> Result<Json<ReviewOutcomeDto>, ApiError> {
    record_review_inner(&s.pool, &ctx, cid, deck_id, card_id, b).await
}
async fn record_review_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, deck_id, card_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<RatingBody>,
) -> Result<Json<ReviewOutcomeDto>, ApiError> {
    record_review_inner(&s.pool, &ctx, cid, deck_id, card_id, b).await
}

async fn due_count(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    due_count_inner(&s.pool, &ctx, cid).await
}
async fn due_count_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    due_count_inner(&s.pool, &ctx, cid).await
}
