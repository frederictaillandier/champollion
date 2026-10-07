//! The `card_forms` table: the form of a card's word learnt at each level
//! above 1.

use champollion_api::CardForm;
use sqlx::PgConnection;

/// Stores the card's form of `form.level`, replacing the one it had.
/// Returns whether it was new or different.
pub async fn upsert(
    conn: &mut PgConnection,
    card_id: i64,
    form: &CardForm,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO card_forms (card_id, level, form, translation) VALUES ($1, $2, $3, $4)
        ON CONFLICT (card_id, level) DO UPDATE
        SET form = EXCLUDED.form, translation = EXCLUDED.translation
        WHERE (card_forms.form, card_forms.translation)
            IS DISTINCT FROM (EXCLUDED.form, EXCLUDED.translation)",
    )
    .bind(card_id)
    .bind(form.level)
    .bind(&form.form)
    .bind(&form.translation)
    .execute(conn)
    .await?;
    Ok(result.rows_affected() > 0)
}
