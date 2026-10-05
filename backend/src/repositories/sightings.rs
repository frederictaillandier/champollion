//! The `sightings` table: each word read in a game, under its card.

use champollion_api::{CardSighting, Sighting};
use sqlx::PgConnection;

/// What [`upsert`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Upsert {
    Added,
    Updated,
    Unchanged,
}

/// A sighting as shown on a card, with the card it belongs to.
#[derive(sqlx::FromRow)]
pub struct CardSightingRow {
    pub card_id: i64,
    pub form: String,
    pub sentence: String,
    pub sentence_translation: String,
    pub definition: String,
}

impl From<CardSightingRow> for CardSighting {
    fn from(r: CardSightingRow) -> Self {
        Self {
            form: r.form,
            sentence: r.sentence,
            sentence_translation: r.sentence_translation,
            definition: r.definition,
        }
    }
}

/// Adds the sighting, unless the card has one of the same form on the same
/// frame: that one then takes the sentence, its translation and the
/// definition sent.
pub async fn upsert(
    conn: &mut PgConnection,
    card_id: i64,
    s: &Sighting,
) -> Result<Upsert, sqlx::Error> {
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
    .bind(card_id)
    .bind(&s.form)
    .bind(&s.sentence)
    .bind(&s.sentence_translation)
    .bind(&s.definition)
    .bind(&s.game)
    .bind(&s.video)
    .bind(s.seconds)
    .bind(&s.frame)
    .fetch_optional(conn)
    .await?;
    Ok(match inserted {
        Some(true) => Upsert::Added,
        Some(false) => Upsert::Updated,
        None => Upsert::Unchanged,
    })
}

/// The sightings of these cards, oldest first.
pub async fn of_cards(
    conn: &mut PgConnection,
    card_ids: &[i64],
) -> Result<Vec<CardSightingRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT card_id, form, sentence, sentence_translation, definition FROM sightings
        WHERE card_id = ANY($1)
        ORDER BY id",
    )
    .bind(card_ids)
    .fetch_all(conn)
    .await
}
