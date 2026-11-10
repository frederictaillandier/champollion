//! The `cards` table: one card per dictionary form, with its review state.

use champollion_api::NewCard;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use crate::schedule::CardState;

/// A card of a session, without its sightings.
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
    pub level: i16,
    /// The form of the card's level and its translation, none at level 1.
    pub form: Option<String>,
    pub form_translation: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StateRow {
    interval_days: f64,
    ease: f64,
    reps: i32,
    lapses: i32,
    level: i16,
    next_level: bool,
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

// A new card was never reviewed (`reps = 0 AND lapses = 0`): it is to learn,
// so not counted due, though its `due` is when it was added.

/// Up to `limit` cards reviewed before, due now and not flagged, in random
/// order.
pub async fn due_reviews(conn: &mut PgConnection, limit: i64) -> Result<Vec<DueCard>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.id, c.lang, c.lemma, c.pos, c.gender, c.translation, c.due, c.reps, c.level,
            f.form, f.translation AS form_translation
        FROM cards c
        LEFT JOIN card_forms f ON f.card_id = c.id AND f.level = c.level
        WHERE c.due <= now() AND (c.reps > 0 OR c.lapses > 0) AND c.flagged_at IS NULL
        ORDER BY random()
        LIMIT $1",
    )
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// Up to `limit` new cards not flagged, in random order.
pub async fn new_cards(conn: &mut PgConnection, limit: i64) -> Result<Vec<DueCard>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.id, c.lang, c.lemma, c.pos, c.gender, c.translation, c.due, c.reps, c.level,
            f.form, f.translation AS form_translation
        FROM cards c
        LEFT JOIN card_forms f ON f.card_id = c.id AND f.level = c.level
        WHERE c.reps = 0 AND c.lapses = 0 AND c.flagged_at IS NULL
        ORDER BY random()
        LIMIT $1",
    )
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// How many cards reviewed before are due now and not flagged.
pub async fn due_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM cards
        WHERE due <= now() AND (reps > 0 OR lapses > 0) AND flagged_at IS NULL",
    )
    .fetch_one(conn)
    .await
}

/// How many new cards are not flagged.
pub async fn new_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM cards WHERE reps = 0 AND lapses = 0 AND flagged_at IS NULL",
    )
    .fetch_one(conn)
    .await
}

/// The card's review state, locked until the end of the transaction, and
/// whether it has a form for the level above. None if there is no such card.
pub async fn lock_state(
    conn: &mut PgConnection,
    id: i64,
) -> Result<Option<(CardState, bool)>, sqlx::Error> {
    let row: Option<StateRow> = sqlx::query_as(
        "SELECT interval_days, ease, reps, lapses, level,
            EXISTS (SELECT 1 FROM card_forms f WHERE f.card_id = c.id AND f.level = c.level + 1)
                AS next_level
        FROM cards c WHERE id = $1 FOR UPDATE OF c",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| {
        let state = CardState {
            interval_days: r.interval_days,
            ease: r.ease,
            reps: r.reps,
            lapses: r.lapses,
            level: r.level,
        };
        (state, r.next_level)
    }))
}

pub async fn set_state(
    conn: &mut PgConnection,
    id: i64,
    state: &CardState,
    due: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cards SET due = $2, interval_days = $3, ease = $4, reps = $5, lapses = $6,
            level = $7
        WHERE id = $1",
    )
    .bind(id)
    .bind(due)
    .bind(state.interval_days)
    .bind(state.ease)
    .bind(state.reps)
    .bind(state.lapses)
    .bind(state.level)
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
