use axum::Json;
use axum::extract::State;
use champollion_api::{ReviewBatch, ReviewBatchResult};

use super::AppState;
use crate::error::Error;

/// `POST /reviews`
pub async fn add(
    State(state): State<AppState>,
    Json(batch): Json<ReviewBatch>,
) -> Result<Json<ReviewBatchResult>, Error> {
    Ok(Json(state.reviews.apply(batch).await?))
}
