//! Data access: one module per table, each function one query. They take a
//! connection, so that services can run several in one transaction.

pub mod cards;
pub mod pronunciations;
pub mod reviews;
pub mod sightings;

use sqlx::PgConnection;

/// Checks that the database answers.
pub async fn ping(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(conn).await?;
    Ok(())
}
