//! The `pronunciations` table: the MP3 of each card's dictionary form.

use sqlx::PgConnection;

/// A card whose dictionary form has no pronunciation yet.
#[derive(sqlx::FromRow)]
pub struct Unspoken {
    pub id: i64,
    pub lang: String,
    pub lemma: String,
}

/// Up to `limit` cards without a pronunciation, leaving out flagged cards and
/// those of `skip`; the soonest due first, as they are reviewed first.
pub async fn missing(
    conn: &mut PgConnection,
    skip: &[i64],
    limit: i64,
) -> Result<Vec<Unspoken>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.id, c.lang, c.lemma FROM cards c
        LEFT JOIN pronunciations p ON p.card_id = c.id
        WHERE p.card_id IS NULL AND c.flagged_at IS NULL AND NOT c.id = ANY($1)
        ORDER BY c.due, c.id
        LIMIT $2",
    )
    .bind(skip)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// Stores the pronunciation of a card, replacing the one it had.
pub async fn upsert(
    conn: &mut PgConnection,
    card_id: i64,
    audio: &[u8],
    voice: &str,
    model: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pronunciations (card_id, audio, voice, model) VALUES ($1, $2, $3, $4)
        ON CONFLICT (card_id) DO UPDATE
        SET audio = $2, voice = $3, model = $4, made_at = now()",
    )
    .bind(card_id)
    .bind(audio)
    .bind(voice)
    .bind(model)
    .execute(conn)
    .await?;
    Ok(())
}

/// The MP3 of a card, if it has one.
pub async fn audio(conn: &mut PgConnection, card_id: i64) -> Result<Option<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar("SELECT audio FROM pronunciations WHERE card_id = $1")
        .bind(card_id)
        .fetch_optional(conn)
        .await
}
