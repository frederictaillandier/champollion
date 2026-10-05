use axum::Json;
use axum::extract::{Query, State};
use champollion_api::{Card, CardBatch, CardBatchResult, DueCount};
use serde::Deserialize;

use super::AppState;
use crate::error::Error;

/// `POST /cards`
pub async fn add(
    State(state): State<AppState>,
    Json(batch): Json<CardBatch>,
) -> Result<Json<CardBatchResult>, Error> {
    Ok(Json(state.cards.add(&batch).await?))
}

#[derive(Deserialize)]
pub struct DueQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// `GET /cards/due?limit=100`
pub async fn due(
    State(state): State<AppState>,
    Query(q): Query<DueQuery>,
) -> Result<Json<Vec<Card>>, Error> {
    Ok(Json(state.cards.due(q.limit).await?))
}

/// `GET /cards/due/count`
pub async fn due_count(State(state): State<AppState>) -> Result<Json<DueCount>, Error> {
    Ok(Json(state.cards.due_count().await?))
}
