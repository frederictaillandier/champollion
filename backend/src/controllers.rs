//! HTTP: each handler reads the request, calls a service and answers.

mod cards;
mod flags;
mod health;
mod reviews;

use axum::Router;
use axum::routing::{get, post};
use sqlx::PgPool;

use crate::services::{
    CardService, FlagService, HealthService, PronunciationService, ReviewService,
};

/// The services, shared by the handlers.
#[derive(Clone)]
pub struct AppState {
    pub health: HealthService,
    pub cards: CardService,
    pub reviews: ReviewService,
    pub flags: FlagService,
    pub pronunciations: PronunciationService,
}

impl AppState {
    pub fn new(db: PgPool, pronunciations: PronunciationService) -> Self {
        Self {
            health: HealthService::new(db.clone()),
            cards: CardService::new(db.clone()),
            reviews: ReviewService::new(db.clone()),
            flags: FlagService::new(db),
            pronunciations,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/cards", post(cards::add))
        .route("/cards/due", get(cards::due))
        .route("/cards/due/count", get(cards::due_count))
        .route("/cards/{id}/audio", get(cards::audio))
        .route("/reviews", post(reviews::add))
        .route("/flags", post(flags::add))
        .with_state(state)
}
