use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use champollion_api::{Card, Rating, ReviewBatch, WordBatch, WordBatchResult};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::schedule::CardState;

pub fn router(db: PgPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/words", post(add_words))
        .route("/cards/due", get(due_cards))
        .route("/reviews", post(add_reviews))
        .with_state(db)
}

/// A database failure, logged and answered with a 500.
struct Error(sqlx::Error);

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        tracing::error!("database: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
    }
}

async fn health(State(db): State<PgPool>) -> Result<&'static str, Error> {
    sqlx::query("SELECT 1").execute(&db).await?;
    Ok("ok")
}

/// Adds the words not known yet, each with a new card. Words already known
/// are ignored, so a batch can be sent again safely.
async fn add_words(
    State(db): State<PgPool>,
    Json(batch): Json<WordBatch>,
) -> Result<Json<WordBatchResult>, Error> {
    let mut tx = db.begin().await?;
    let mut added = 0;
    for w in &batch.words {
        added += sqlx::query(
            "WITH w AS (
                INSERT INTO words (lang, text, sentence, game, video, seconds, frame)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (lang, text) DO NOTHING
                RETURNING id
            )
            INSERT INTO cards (word_id) SELECT id FROM w",
        )
        .bind(&w.lang)
        .bind(&w.text)
        .bind(&w.sentence)
        .bind(&w.game)
        .bind(&w.video)
        .bind(w.seconds)
        .bind(&w.frame)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    if added > 0 {
        tracing::info!("added {added} of {} words", batch.words.len());
    }
    Ok(Json(WordBatchResult { added }))
}

#[derive(Deserialize)]
struct DueQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// Cards due now, the most overdue first.
async fn due_cards(
    State(db): State<PgPool>,
    Query(q): Query<DueQuery>,
) -> Result<Json<Vec<Card>>, Error> {
    let rows = sqlx::query(
        "SELECT w.id, w.lang, w.text, w.sentence, w.game, c.due, c.reps
        FROM cards c JOIN words w ON w.id = c.word_id
        WHERE c.due <= now()
        ORDER BY c.due
        LIMIT $1",
    )
    .bind(q.limit.clamp(1, 1000))
    .fetch_all(&db)
    .await?;
    let cards = rows
        .iter()
        .map(|r| {
            Ok(Card {
                word_id: r.try_get("id")?,
                lang: r.try_get("lang")?,
                text: r.try_get("text")?,
                sentence: r.try_get("sentence")?,
                game: r.try_get("game")?,
                due: r.try_get("due")?,
                reps: r.try_get("reps")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;
    Ok(Json(cards))
}

#[derive(Serialize)]
struct ReviewBatchResult {
    /// Reviews not received before.
    applied: u64,
}

/// Records reviews and reschedules their cards. A review already received
/// (same id) or of an unknown word is skipped.
async fn add_reviews(
    State(db): State<PgPool>,
    Json(batch): Json<ReviewBatch>,
) -> Result<Json<ReviewBatchResult>, Error> {
    let mut reviews = batch.reviews;
    // Reviews made offline may arrive in any order.
    reviews.sort_by_key(|r| r.reviewed_at);
    let mut tx = db.begin().await?;
    let mut applied = 0;
    for review in &reviews {
        let card = sqlx::query(
            "SELECT interval_days, ease, reps, lapses FROM cards WHERE word_id = $1 FOR UPDATE",
        )
        .bind(review.word_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(card) = card else {
            tracing::warn!("review of unknown word {}", review.word_id);
            continue;
        };
        let inserted = sqlx::query(
            "INSERT INTO reviews (id, word_id, rating, reviewed_at) VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO NOTHING",
        )
        .bind(review.id)
        .bind(review.word_id)
        .bind(rating_name(review.rating))
        .bind(review.reviewed_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            continue;
        }
        let state = CardState {
            interval_days: card.try_get("interval_days")?,
            ease: card.try_get("ease")?,
            reps: card.try_get("reps")?,
            lapses: card.try_get("lapses")?,
        };
        let (next, due) = state.review(review.rating, review.reviewed_at);
        sqlx::query(
            "UPDATE cards SET due = $2, interval_days = $3, ease = $4, reps = $5, lapses = $6
            WHERE word_id = $1",
        )
        .bind(review.word_id)
        .bind(due)
        .bind(next.interval_days)
        .bind(next.ease)
        .bind(next.reps)
        .bind(next.lapses)
        .execute(&mut *tx)
        .await?;
        applied += 1;
    }
    tx.commit().await?;
    Ok(Json(ReviewBatchResult { applied }))
}

fn rating_name(rating: Rating) -> &'static str {
    match rating {
        Rating::Again => "again",
        Rating::Hard => "hard",
        Rating::Good => "good",
        Rating::Easy => "easy",
    }
}
