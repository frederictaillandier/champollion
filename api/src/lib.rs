//! JSON types exchanged with the champollion backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A word read in a game, sent by the daemon (`POST /words`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewWord {
    /// Tesseract language code, e.g. `ces`.
    pub lang: String,
    /// Lowercase spelling, the word's identity within its language.
    pub text: String,
    /// The sentence it was first read in.
    pub sentence: String,
    /// Game slug, e.g. `1771300-kingdom-come-deliverance-ii`.
    pub game: String,
    /// Recording it was first read in, relative to the recordings directory.
    pub video: String,
    /// Position in that recording.
    pub seconds: f64,
    /// Saved frame showing it, relative to the game's text directory.
    pub frame: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordBatch {
    pub words: Vec<NewWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordBatchResult {
    /// Words the backend did not know yet.
    pub added: u64,
}

/// A word to review (`GET /cards/due`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub word_id: i64,
    pub lang: String,
    pub text: String,
    pub sentence: String,
    pub game: String,
    pub due: DateTime<Utc>,
    pub reps: i32,
}

/// How well a card was remembered, as in Anki.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

/// A review made on a device (`POST /reviews`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    /// Chosen by the device, so sending the same review twice applies it once.
    pub id: Uuid,
    pub word_id: i64,
    pub rating: Rating,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBatch {
    pub reviews: Vec<Review>,
}
