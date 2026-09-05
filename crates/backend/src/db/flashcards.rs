//! Flashcards (learning-suite Cycle 6): teacher decks + per-student SM-2
//! review state. The scheduling math in `next_review` mirrors kinetics
//! ui-learn's `next_review` exactly (Again lapses to 0 days/-0.2 ease;
//! Hard ×1.2/-0.15; Good graduates 1 → 6 → interval×ease; Easy ×ease×1.3
//! bonus/+0.15; ease floor 1.3) — the server is authoritative, the frontend
//! helper only previews.

use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

pub const MIN_EASE: f32 = 1.3;
/// New (never-reviewed) cards introduced per review session, on top of all
/// due cards.
pub const NEW_CARDS_PER_SESSION: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewState {
    pub interval_days: f32,
    pub ease: f32,
    pub repetitions: i32,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            interval_days: 0.0,
            ease: 2.5,
            repetitions: 0,
        }
    }
}

/// Advance one review. Mirrors kinetics `next_review` (see module docs).
pub fn next_review(state: ReviewState, rating: &str) -> ReviewState {
    let ease = state.ease.max(MIN_EASE);
    match rating {
        "again" => ReviewState {
            interval_days: 0.0,
            ease: (ease - 0.2).max(MIN_EASE),
            repetitions: 0,
        },
        "hard" => ReviewState {
            interval_days: (state.interval_days * 1.2).max(1.0),
            ease: (ease - 0.15).max(MIN_EASE),
            repetitions: state.repetitions + 1,
        },
        "easy" => ReviewState {
            interval_days: match state.repetitions {
                0 => 4.0,
                _ => state.interval_days * ease * 1.3,
            },
            ease: ease + 0.15,
            repetitions: state.repetitions + 1,
        },
        // "good" (and any unexpected value, which handlers reject upstream).
        _ => ReviewState {
            interval_days: match state.repetitions {
                0 => 1.0,
                1 => 6.0,
                _ => state.interval_days * ease,
            },
            ease,
            repetitions: state.repetitions + 1,
        },
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct DeckRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub card_count: i64,
}

pub async fn list_decks(
    conn: &mut PgConnection,
    course_id: Uuid,
    include_drafts: bool,
) -> sqlx::Result<Vec<DeckRow>> {
    sqlx::query_as(
        "SELECT d.id, d.course_id, d.title, d.description, d.status, d.created_at,
                (SELECT count(*) FROM flashcards f WHERE f.deck_id = d.id) AS card_count
           FROM flashcard_decks d
          WHERE d.course_id = $1 AND (d.status = 'published' OR $2)
          ORDER BY d.created_at ASC",
    )
    .bind(course_id)
    .bind(include_drafts)
    .fetch_all(conn)
    .await
}

pub async fn fetch_deck(conn: &mut PgConnection, deck_id: Uuid) -> sqlx::Result<Option<DeckRow>> {
    sqlx::query_as(
        "SELECT d.id, d.course_id, d.title, d.description, d.status, d.created_at,
                (SELECT count(*) FROM flashcards f WHERE f.deck_id = d.id) AS card_count
           FROM flashcard_decks d WHERE d.id = $1",
    )
    .bind(deck_id)
    .fetch_optional(conn)
    .await
}

pub async fn create_deck(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    title: &str,
    description: Option<&str>,
    created_by: Uuid,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO flashcard_decks (tenant_id, course_id, title, description, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(title)
    .bind(description)
    .bind(created_by)
    .fetch_one(&mut **tx)
    .await
}

pub async fn patch_deck(
    tx: &mut Transaction<'_, Postgres>,
    deck_id: Uuid,
    title: Option<&str>,
    description: Option<&str>,
    status: Option<&str>,
) -> sqlx::Result<bool> {
    let n = sqlx::query(
        "UPDATE flashcard_decks SET
            title = COALESCE($2, title),
            description = COALESCE($3, description),
            status = COALESCE($4, status)
          WHERE id = $1",
    )
    .bind(deck_id)
    .bind(title)
    .bind(description)
    .bind(status)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(n > 0)
}

pub async fn delete_deck(tx: &mut Transaction<'_, Postgres>, deck_id: Uuid) -> sqlx::Result<bool> {
    let n = sqlx::query("DELETE FROM flashcard_decks WHERE id = $1")
        .bind(deck_id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(n > 0)
}

#[derive(Debug, sqlx::FromRow)]
pub struct CardRow {
    pub id: Uuid,
    pub deck_id: Uuid,
    pub position: i32,
    pub front: String,
    pub back: String,
}

pub async fn list_cards(conn: &mut PgConnection, deck_id: Uuid) -> sqlx::Result<Vec<CardRow>> {
    sqlx::query_as(
        "SELECT id, deck_id, position, front, back FROM flashcards
          WHERE deck_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(deck_id)
    .fetch_all(conn)
    .await
}

pub async fn create_card(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    deck_id: Uuid,
    front: &str,
    back: &str,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "INSERT INTO flashcards (tenant_id, deck_id, position, front, back)
         SELECT $1, d.id,
                COALESCE((SELECT max(f.position) + 10
                            FROM flashcards f
                           WHERE f.tenant_id = $1 AND f.deck_id = d.id), 10),
                $4, $5
           FROM flashcard_decks d
          WHERE d.tenant_id = $1 AND d.course_id = $2 AND d.id = $3
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(deck_id)
    .bind(front)
    .bind(back)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn patch_card(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    deck_id: Uuid,
    card_id: Uuid,
    front: Option<&str>,
    back: Option<&str>,
) -> sqlx::Result<bool> {
    let n = sqlx::query(
        "UPDATE flashcards f
            SET front = COALESCE($5, f.front), back = COALESCE($6, f.back)
          WHERE f.tenant_id = $1 AND f.deck_id = $3 AND f.id = $4
            AND EXISTS (
                SELECT 1 FROM flashcard_decks d
                 WHERE d.tenant_id = $1 AND d.course_id = $2 AND d.id = $3
            )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(deck_id)
    .bind(card_id)
    .bind(front)
    .bind(back)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(n > 0)
}

pub async fn delete_card(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    deck_id: Uuid,
    card_id: Uuid,
) -> sqlx::Result<bool> {
    let n = sqlx::query(
        "DELETE FROM flashcards f
          USING flashcard_decks d
          WHERE f.tenant_id = $1 AND f.deck_id = $3 AND f.id = $4
            AND d.tenant_id = $1 AND d.course_id = $2 AND d.id = $3",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(deck_id)
    .bind(card_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(n > 0)
}

#[derive(Debug, sqlx::FromRow)]
pub struct DueCardRow {
    pub id: Uuid,
    pub front: String,
    pub back: String,
    pub ease: Option<f32>,
    pub interval_days: Option<f32>,
    pub repetitions: Option<i32>,
}

/// The student's review queue for a deck: every due card (due_date ≤ today)
/// plus up to `NEW_CARDS_PER_SESSION` never-reviewed cards, in deck order.
pub async fn due_cards(
    conn: &mut PgConnection,
    deck_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<DueCardRow>> {
    sqlx::query_as(
        "(SELECT f.id, f.front, f.back, rs.ease, rs.interval_days, rs.repetitions
            FROM flashcards f
            JOIN flashcard_review_state rs
              ON rs.card_id = f.id AND rs.user_id = $2
           WHERE f.deck_id = $1 AND rs.due_date <= CURRENT_DATE
           ORDER BY f.position ASC)
         UNION ALL
         (SELECT f.id, f.front, f.back, NULL::real, NULL::real, NULL::int
            FROM flashcards f
           WHERE f.deck_id = $1
             AND NOT EXISTS (SELECT 1 FROM flashcard_review_state rs
                              WHERE rs.card_id = f.id AND rs.user_id = $2)
           ORDER BY f.position ASC
           LIMIT $3)",
    )
    .bind(deck_id)
    .bind(user_id)
    .bind(NEW_CARDS_PER_SESSION)
    .fetch_all(conn)
    .await
}

/// Count of cards due today across all published decks of a course (for the
/// course-card / dashboard badge).
pub async fn due_count_for_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT
            (SELECT count(*)
               FROM flashcards f
               JOIN flashcard_decks d ON d.id = f.deck_id
               JOIN flashcard_review_state rs ON rs.card_id = f.id AND rs.user_id = $2
              WHERE d.course_id = $1 AND d.status = 'published'
                AND rs.due_date <= CURRENT_DATE)
          + (SELECT LEAST(count(*), $3)
               FROM flashcards f
               JOIN flashcard_decks d ON d.id = f.deck_id
              WHERE d.course_id = $1 AND d.status = 'published'
                AND NOT EXISTS (SELECT 1 FROM flashcard_review_state rs
                                 WHERE rs.card_id = f.id AND rs.user_id = $2))",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(NEW_CARDS_PER_SESSION)
    .fetch_one(conn)
    .await
}

/// Record one review: load (or default) the state, advance it via
/// `next_review`, and upsert with the new due date (today + ceil(interval)).
/// Returns the new state.
pub async fn record_review(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    card_id: Uuid,
    user_id: Uuid,
    rating: &str,
) -> sqlx::Result<ReviewState> {
    let current: Option<(f32, f32, i32)> = sqlx::query_as(
        "SELECT ease, interval_days, repetitions FROM flashcard_review_state
          WHERE card_id = $1 AND user_id = $2",
    )
    .bind(card_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let state = current
        .map(|(ease, interval_days, repetitions)| ReviewState {
            ease,
            interval_days,
            repetitions,
        })
        .unwrap_or_default();
    let next = next_review(state, rating);
    // Due in ceil(interval) days; "again" (0.0) stays due today.
    let due_in_days = next.interval_days.ceil() as i32;
    sqlx::query(
        "INSERT INTO flashcard_review_state
            (tenant_id, card_id, user_id, ease, interval_days, repetitions, due_date, last_rating, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, CURRENT_DATE + $7, $8, now())
         ON CONFLICT (card_id, user_id) DO UPDATE SET
            ease = EXCLUDED.ease,
            interval_days = EXCLUDED.interval_days,
            repetitions = EXCLUDED.repetitions,
            due_date = EXCLUDED.due_date,
            last_rating = EXCLUDED.last_rating,
            updated_at = now()",
    )
    .bind(tenant_id)
    .bind(card_id)
    .bind(user_id)
    .bind(next.ease)
    .bind(next.interval_days)
    .bind(next.repetitions)
    .bind(due_in_days)
    .bind(rating)
    .execute(&mut **tx)
    .await?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::{next_review, ReviewState, MIN_EASE};

    #[test]
    fn mirrors_kinetics_sm2_semantics() {
        let fresh = ReviewState::default();
        // Good graduates 1 → 6 → interval×ease.
        let g1 = next_review(fresh, "good");
        assert_eq!(g1.interval_days, 1.0);
        assert_eq!(g1.repetitions, 1);
        let g2 = next_review(g1, "good");
        assert_eq!(g2.interval_days, 6.0);
        let g3 = next_review(g2, "good");
        assert!(
            (g3.interval_days - 15.0).abs() < 0.01,
            "{}",
            g3.interval_days
        );

        // Again lapses and floors ease.
        let lapsed = next_review(
            ReviewState {
                interval_days: 30.0,
                ease: 1.4,
                repetitions: 5,
            },
            "again",
        );
        assert_eq!(lapsed.interval_days, 0.0);
        assert_eq!(lapsed.repetitions, 0);
        assert_eq!(lapsed.ease, MIN_EASE);

        // Hard grows 1.2× with a 1-day floor; Easy first review jumps to 4.
        let hard = next_review(fresh, "hard");
        assert_eq!(hard.interval_days, 1.0);
        let easy = next_review(fresh, "easy");
        assert_eq!(easy.interval_days, 4.0);
        assert!((easy.ease - 2.65).abs() < 0.001);
    }
}
