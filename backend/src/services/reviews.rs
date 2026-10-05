use champollion_api::{ReviewBatch, ReviewBatchResult};
use sqlx::PgPool;

use crate::error::Error;
use crate::repositories::{cards, reviews};

#[derive(Clone)]
pub struct ReviewService {
    db: PgPool,
}

impl ReviewService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Records reviews and reschedules their cards. A review already
    /// received (same id) or of an unknown card is skipped.
    pub async fn apply(&self, batch: ReviewBatch) -> Result<ReviewBatchResult, Error> {
        let mut reviews = batch.reviews;
        // Reviews made offline may arrive in any order.
        reviews.sort_by_key(|r| r.reviewed_at);
        let mut tx = self.db.begin().await?;
        let mut applied = 0;
        for review in &reviews {
            let Some(state) = cards::lock_state(&mut tx, review.card_id).await? else {
                tracing::warn!("review of unknown card {}", review.card_id);
                continue;
            };
            if !reviews::insert(&mut tx, review).await? {
                continue;
            }
            let (next, due) = state.review(review.rating, review.reviewed_at);
            cards::set_state(&mut tx, review.card_id, &next, due).await?;
            applied += 1;
        }
        tx.commit().await?;
        Ok(ReviewBatchResult { applied })
    }
}
