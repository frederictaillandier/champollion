use axum::extract::State;

use super::AppState;
use crate::error::Error;

/// `GET /health`
pub async fn health(State(state): State<AppState>) -> Result<&'static str, Error> {
    state.health.check().await?;
    Ok("ok")
}
