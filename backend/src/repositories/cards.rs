//! The `cards` table: one card per dictionary form, with its review state.

use champollion_api::NewCard;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use crate::schedule::CardState;

/// A card due for review, without its sightings.
#[derive(sqlx::FromRow)]
pub struct DueCard {
    pub id: i64,
    pub lang: String,
    pub lemma: String,
    pub pos: String,
    pub gender: String,
    pub translation: String,
    pub due: DateTime<Utc>,
    pub reps: i32,
}

#[derive(sqlx::FromRow)]
struct StateRow {
    interval_days: f64,
    ease: f64,
    reps: i32,
    lapses: i32,
}

/// Adds the card, unless one has the same language, lemma and part of
/// speech. Returns its id if added.
pub async fn insert(conn: &mut PgConnection, card: &NewCard) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
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
    .fetch_optional(conn)
    .await
}

/// The id of the card with this language, lemma and part of speech.
pub async fn id_of(conn: &mut PgConnection, card: &NewCard) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM cards WHERE lang = $1 AND lemma = $2 AND pos = $3")
        .bind(&card.lang)
        .bind(&card.lemma)
        .bind(&card.pos)
        .fetch_one(conn)
        .await
}

/// Up to `limit` cards due now and not flagged: those already reviewed
/// first, then new ones, each in random order.
pub async fn due(conn: &mut PgConnection, limit: i64) -> Result<Vec<DueCard>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, lang, lemma, pos, gender, translation, due, reps FROM cards
        WHERE due <= now() AND flagged_at IS NULL
        ORDER BY (reps > 0 OR lapses > 0) DESC, random()
        LIMIT $1",
    )
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// How many cards are due now and not flagged.
pub async fn due_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM cards WHERE due <= now() AND flagged_at IS NULL")
        .fetch_one(conn)
        .await
}

/// The card's review state, locked until the end of the transaction. None
/// if there is no such card.
pub async fn lock_state(
    conn: &mut PgConnection,
    id: i64,
) -> Result<Option<CardState>, sqlx::Error> {
    let row: Option<StateRow> = sqlx::query_as(
        "SELECT interval_days, ease, reps, lapses FROM cards WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| CardState {
        interval_days: r.interval_days,
        ease: r.ease,
        reps: r.reps,
        lapses: r.lapses,
    }))
}

pub async fn set_state(
    conn: &mut PgConnection,
    id: i64,
    state: &CardState,
    due: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cards SET due = $2, interval_days = $3, ease = $4, reps = $5, lapses = $6
        WHERE id = $1",
    )
    .bind(id)
    .bind(due)
    .bind(state.interval_days)
    .bind(state.ease)
    .bind(state.reps)
    .bind(state.lapses)
    .execute(conn)
    .await?;
    Ok(())
}

/// Flags the card, unless it already is. Returns whether it was flagged now.
pub async fn flag(
    conn: &mut PgConnection,
    id: i64,
    at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("UPDATE cards SET flagged_at = $2 WHERE id = $1 AND flagged_at IS NULL")
            .bind(id)
            .bind(at)
            .execute(conn)
            .await?;
    Ok(result.rows_affected() > 0)
}
