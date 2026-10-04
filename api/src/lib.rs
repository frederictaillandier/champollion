//! JSON types exchanged with the champollion backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A word to learn, in its dictionary form. Words read in the game are
/// grouped under it: `králem` and `králi` are sightings of `král`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCard {
    /// Tesseract language code, e.g. `ces`.
    pub lang: String,
    /// Dictionary form, e.g. `král`, `namazat se`.
    pub lemma: String,
    /// Part of speech: `noun`, `verb`, `adjective`...
    pub pos: String,
    /// `m`, `f` or `n` for nouns, empty otherwise.
    pub gender: String,
    /// Short English translation.
    pub translation: String,
    pub sighting: Sighting,
}

/// Where and how a word was read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sighting {
    /// The word as read, lowercase, e.g. `králem`.
    pub form: String,
    pub sentence: String,
    pub sentence_translation: String,
    /// What the word means in this sentence, in English.
    pub definition: String,
    /// Game slug, e.g. `1771300-kingdom-come-deliverance-ii`.
    pub game: String,
    /// Recording it was read in, relative to the recordings directory.
    pub video: String,
    /// Position in that recording.
    pub seconds: f64,
    /// Saved frame showing it, relative to the game's text directory.
    pub frame: String,
}

/// `POST /cards`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardBatch {
    pub cards: Vec<NewCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardBatchResult {
    /// Cards the backend did not have yet.
    pub added_cards: u64,
    /// New sightings, of new or known cards.
    pub added_sightings: u64,
    /// Known sightings whose definition or sentence translation changed.
    #[serde(default)]
    pub updated_sightings: u64,
}

/// A card to review (`GET /cards/due`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: i64,
    pub lang: String,
    pub lemma: String,
    pub pos: String,
    pub gender: String,
    pub translation: String,
    pub due: DateTime<Utc>,
    /// Successful reviews in a row; 0 for a new card.
    pub reps: i32,
    /// Oldest first.
    pub sightings: Vec<CardSighting>,
}

/// A sighting as shown on a card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSighting {
    pub form: String,
    pub sentence: String,
    pub sentence_translation: String,
    pub definition: String,
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
    pub card_id: i64,
    pub rating: Rating,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBatch {
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewBatchResult {
    /// Reviews not received before.
    pub applied: u64,
}

/// A card flagged on a device as having an issue: it is no longer reviewed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flag {
    pub card_id: i64,
    pub flagged_at: DateTime<Utc>,
}

/// `POST /flags`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagBatch {
    pub flags: Vec<Flag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagBatchResult {
    /// Cards not flagged before.
    pub applied: u64,
}
