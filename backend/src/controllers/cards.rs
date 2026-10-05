use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use champollion_api::{Card, CardBatch, CardBatchResult, DueCount};
use serde::Deserialize;

use super::AppState;
use crate::error::Error;

/// `POST /cards`
pub async fn add(
    State(state): State<AppState>,
    Json(batch): Json<CardBatch>,
) -> Result<Json<CardBatchResult>, Error> {
    let result = state.cards.add(&batch).await?;
    if result.added_cards > 0 {
        state.pronunciations.wake();
    }
    Ok(Json(result))
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

/// `GET /cards/{id}/audio`: the MP3 of the card's dictionary form, or 404
/// while it is not made.
pub async fn audio(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Response, Error> {
    Ok(match state.pronunciations.audio(id).await? {
        Some(mp3) => ([(header::CONTENT_TYPE, "audio/mpeg")], mp3).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}
