use sqlx::PgPool;

use crate::error::Error;
use crate::repositories;

#[derive(Clone)]
pub struct HealthService {
    db: PgPool,
}

impl HealthService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Fails if the database does not answer.
    pub async fn check(&self) -> Result<(), Error> {
        let mut conn = self.db.acquire().await?;
        repositories::ping(&mut conn).await?;
        Ok(())
    }
}
