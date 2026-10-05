use axum::Json;
use axum::extract::State;
use champollion_api::{FlagBatch, FlagBatchResult};

use super::AppState;
use crate::error::Error;

/// `POST /flags`
pub async fn add(
    State(state): State<AppState>,
    Json(batch): Json<FlagBatch>,
) -> Result<Json<FlagBatchResult>, Error> {
    Ok(Json(state.flags.flag(&batch).await?))
}
