//! The `reviews` table: every rating received, as history.

use champollion_api::{Rating, Review};
use sqlx::PgConnection;

/// Records the review, unless one with its id was. Returns whether it was
/// recorded now.
pub async fn insert(conn: &mut PgConnection, review: &Review) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO reviews (id, card_id, rating, reviewed_at) VALUES ($1, $2, $3, $4)
        ON CONFLICT (id) DO NOTHING",
    )
    .bind(review.id)
    .bind(review.card_id)
    .bind(rating_name(review.rating))
    .bind(review.reviewed_at)
    .execute(conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

fn rating_name(rating: Rating) -> &'static str {
    match rating {
        Rating::Failed => "failed",
        Rating::Succeeded => "succeeded",
    }
}
