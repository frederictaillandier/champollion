use champollion_api::{FlagBatch, FlagBatchResult};
use sqlx::PgPool;

use crate::error::Error;
use crate::repositories::cards;

#[derive(Clone)]
pub struct FlagService {
    db: PgPool,
}

impl FlagService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Flags cards, keeping the first flag of each. Unknown cards are
    /// skipped.
    pub async fn flag(&self, batch: &FlagBatch) -> Result<FlagBatchResult, Error> {
        let mut tx = self.db.begin().await?;
        let mut applied = 0;
        for flag in &batch.flags {
            if cards::flag(&mut tx, flag.card_id, flag.flagged_at).await? {
                applied += 1;
            }
        }
        tx.commit().await?;
        if applied > 0 {
            tracing::info!("flagged {applied} cards");
        }
        Ok(FlagBatchResult { applied })
    }
}
