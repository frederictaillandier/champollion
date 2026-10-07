//! The `pronunciations` table: the MP3 of each card's dictionary form, and
//! of its form of each level above 1.

use sqlx::PgConnection;

/// A card's text with no pronunciation yet, or one of an older text.
#[derive(sqlx::FromRow)]
pub struct Unspoken {
    pub id: i64,
    pub level: i16,
    pub lang: String,
    /// The lemma at level 1, the level's form above.
    pub text: String,
}

/// Up to `limit` texts to speak, leaving out flagged cards and the levels
/// of `skip`; the soonest due first, as they are reviewed first. Every
/// level is spoken, not only the card's, so it is ready once the card moves
/// up.
pub async fn missing(
    conn: &mut PgConnection,
    skip: &[(i64, i16)],
    limit: i64,
) -> Result<Vec<Unspoken>, sqlx::Error> {
    let (skip_ids, skip_levels): (Vec<i64>, Vec<i16>) = skip.iter().copied().unzip();
    sqlx::query_as(
        "SELECT c.id, t.level, c.lang, t.text FROM cards c
        CROSS JOIN LATERAL (
            SELECT 1::smallint AS level, c.lemma AS text
            UNION ALL
            SELECT f.level, f.form FROM card_forms f WHERE f.card_id = c.id
        ) t
        LEFT JOIN pronunciations p ON p.card_id = c.id AND p.level = t.level
        WHERE (p.card_id IS NULL OR p.text <> t.text) AND c.flagged_at IS NULL
            AND (c.id, t.level) NOT IN (SELECT * FROM unnest($1::bigint[], $2::smallint[]))
        ORDER BY c.due, c.id, t.level
        LIMIT $3",
    )
    .bind(skip_ids)
    .bind(skip_levels)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// Stores the pronunciation of a card's level, replacing the one it had.
pub async fn upsert(
    conn: &mut PgConnection,
    card: &Unspoken,
    audio: &[u8],
    voice: &str,
    model: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pronunciations (card_id, level, text, audio, voice, model)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (card_id, level) DO UPDATE
        SET text = $3, audio = $4, voice = $5, model = $6, made_at = now()",
    )
    .bind(card.id)
    .bind(card.level)
    .bind(&card.text)
    .bind(audio)
    .bind(voice)
    .bind(model)
    .execute(conn)
    .await?;
    Ok(())
}

/// The MP3 of a card's level, if it has one.
pub async fn audio(
    conn: &mut PgConnection,
    card_id: i64,
    level: i16,
) -> Result<Option<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar("SELECT audio FROM pronunciations WHERE card_id = $1 AND level = $2")
        .bind(card_id)
        .bind(level)
        .fetch_optional(conn)
        .await
}
