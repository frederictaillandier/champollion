use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use champollion_api::{
    Card, CardBatch, CardBatchResult, CardSighting, FlagBatch, FlagBatchResult, Rating,
    ReviewBatch, ReviewBatchResult,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};

use crate::schedule::CardState;

pub fn router(db: PgPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/cards", post(add_cards))
        .route("/cards/due", get(due_cards))
        .route("/reviews", post(add_reviews))
        .route("/flags", post(add_flags))
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

/// Adds each card unless its lemma is known, then its sighting unless the
/// same form was already seen on the same frame, so a batch can be sent
/// again safely. A known sighting gets the sentence, its translation and the
/// definition sent, so cards made again with a better prompt replace them.
async fn add_cards(
    State(db): State<PgPool>,
    Json(batch): Json<CardBatch>,
) -> Result<Json<CardBatchResult>, Error> {
    let mut tx = db.begin().await?;
    let mut result = CardBatchResult {
        added_cards: 0,
        added_sightings: 0,
        updated_sightings: 0,
    };
    for card in &batch.cards {
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO cards (lang, lemma, pos, gender, translation)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (lang, lemma, pos) DO NOTHING
            RETURNING id",
        )
        .bind(&card.lang)
        .bind(&card.lemma)
        .bind(&card.pos)
        .bind(&card.gender)
        .bind(&card.translation)
        .fetch_optional(&mut *tx)
        .await?;
        let id = match inserted {
            Some(id) => {
                result.added_cards += 1;
                id
            }
            None => {
                sqlx::query_scalar(
                    "SELECT id FROM cards WHERE lang = $1 AND lemma = $2 AND pos = $3",
                )
                .bind(&card.lang)
                .bind(&card.lemma)
                .bind(&card.pos)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        let s = &card.sighting;
        // xmax is 0 for a row just inserted, not for one updated.
        let inserted: Option<bool> = sqlx::query_scalar(
            "INSERT INTO sightings
                (card_id, form, sentence, sentence_translation, definition, game, video, seconds, frame)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (card_id, form, frame) DO UPDATE SET
                sentence = EXCLUDED.sentence,
                sentence_translation = EXCLUDED.sentence_translation,
                definition = EXCLUDED.definition
            WHERE (sightings.sentence, sightings.sentence_translation, sightings.definition)
                IS DISTINCT FROM
                (EXCLUDED.sentence, EXCLUDED.sentence_translation, EXCLUDED.definition)
            RETURNING xmax = 0",
        )
        .bind(id)
        .bind(&s.form)
        .bind(&s.sentence)
        .bind(&s.sentence_translation)
        .bind(&s.definition)
        .bind(&s.game)
        .bind(&s.video)
        .bind(s.seconds)
        .bind(&s.frame)
        .fetch_optional(&mut *tx)
        .await?;
        match inserted {
            Some(true) => result.added_sightings += 1,
            Some(false) => result.updated_sightings += 1,
            None => {}
        }
    }
    tx.commit().await?;
    if result.added_sightings > 0 || result.updated_sightings > 0 {
        tracing::info!(
            "added {} cards and {} sightings, updated {} sightings, from {}",
            result.added_cards,
            result.added_sightings,
            result.updated_sightings,
            batch.cards.len()
        );
    }
    Ok(Json(result))
}

#[derive(Deserialize)]
struct DueQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// Cards due now with their sightings: those already reviewed first, then
/// new ones, each in random order. Flagged cards are left out.
async fn due_cards(
    State(db): State<PgPool>,
    Query(q): Query<DueQuery>,
) -> Result<Json<Vec<Card>>, Error> {
    let rows = sqlx::query(
        "SELECT id, lang, lemma, pos, gender, translation, due, reps FROM cards
        WHERE due <= now() AND flagged_at IS NULL
        ORDER BY (reps > 0 OR lapses > 0) DESC, random()
        LIMIT $1",
    )
    .bind(q.limit.clamp(1, 1000))
    .fetch_all(&db)
    .await?;
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| r.try_get("id"))
        .collect::<Result<_, _>>()?;

    let mut sightings: HashMap<i64, Vec<CardSighting>> = HashMap::new();
    let sighting_rows = sqlx::query(
        "SELECT card_id, form, sentence, sentence_translation, definition FROM sightings
        WHERE card_id = ANY($1)
        ORDER BY id",
    )
    .bind(&ids)
    .fetch_all(&db)
    .await?;
    for r in &sighting_rows {
        sightings
            .entry(r.try_get("card_id")?)
            .or_default()
            .push(CardSighting {
                form: r.try_get("form")?,
                sentence: r.try_get("sentence")?,
                sentence_translation: r.try_get("sentence_translation")?,
                definition: r.try_get("definition")?,
            });
    }

    let cards = rows
        .iter()
        .map(|r| {
            let id: i64 = r.try_get("id")?;
            Ok(Card {
                id,
                lang: r.try_get("lang")?,
                lemma: r.try_get("lemma")?,
                pos: r.try_get("pos")?,
                gender: r.try_get("gender")?,
                translation: r.try_get("translation")?,
                due: r.try_get("due")?,
                reps: r.try_get("reps")?,
                sightings: sightings.remove(&id).unwrap_or_default(),
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;
    Ok(Json(cards))
}

/// Records reviews and reschedules their cards. A review already received
/// (same id) or of an unknown card is skipped.
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
            "SELECT interval_days, ease, reps, lapses FROM cards WHERE id = $1 FOR UPDATE",
        )
        .bind(review.card_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(card) = card else {
            tracing::warn!("review of unknown card {}", review.card_id);
            continue;
        };
        let inserted = sqlx::query(
            "INSERT INTO reviews (id, card_id, rating, reviewed_at) VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO NOTHING",
        )
        .bind(review.id)
        .bind(review.card_id)
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
            WHERE id = $1",
        )
        .bind(review.card_id)
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

/// Flags cards, keeping the first flag of each. Unknown cards are skipped.
async fn add_flags(
    State(db): State<PgPool>,
    Json(batch): Json<FlagBatch>,
) -> Result<Json<FlagBatchResult>, Error> {
    let mut tx = db.begin().await?;
    let mut applied = 0;
    for flag in &batch.flags {
        applied += sqlx::query(
            "UPDATE cards SET flagged_at = $2 WHERE id = $1 AND flagged_at IS NULL",
        )
        .bind(flag.card_id)
        .bind(flag.flagged_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    if applied > 0 {
        tracing::info!("flagged {applied} cards");
    }
    Ok(Json(FlagBatchResult { applied }))
}

fn rating_name(rating: Rating) -> &'static str {
    match rating {
        Rating::Failed => "failed",
        Rating::Succeeded => "succeeded",
    }
}
