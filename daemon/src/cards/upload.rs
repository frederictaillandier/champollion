//! Sends a game's `cards.jsonl` to the backend.

use std::path::Path;
use std::time::Duration;

use champollion_api::{CardBatch, CardBatchResult, NewCard};

use super::Pending;

/// Cards per request.
const BATCH: usize = 200;
/// Next to `cards.jsonl`: how many of its bytes the backend has.
const PROGRESS_FILE: &str = "uploaded.txt";

pub struct Uploader {
    url: String,
    agent: ureq::Agent,
}

impl Uploader {
    pub fn new(backend_url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            url: format!("{backend_url}/cards"),
            agent,
        }
    }

    /// Sends the cards added since the last upload. Sending a card twice is
    /// harmless: the backend keeps the cards it has, and only takes the new
    /// definition and sentence translation of the sightings it has.
    pub fn upload(&mut self, dir: &Path) -> Result<(), String> {
        let Some(mut pending) = Pending::read(dir, "cards.jsonl", PROGRESS_FILE) else {
            return Ok(());
        };
        let game = dir.file_name().unwrap_or_default().to_string_lossy();
        for chunk in pending.lines().chunks(BATCH) {
            let cards = chunk
                .iter()
                .filter_map(|line| match serde_json::from_slice::<NewCard>(line) {
                    Ok(card) => Some(card),
                    Err(e) => {
                        tracing::warn!("skipping a broken line of {game}/cards.jsonl: {e}");
                        None
                    }
                })
                .collect();
            let result: CardBatchResult = self
                .agent
                .post(&self.url)
                .send_json(CardBatch { cards })
                .and_then(|mut r| r.body_mut().read_json())
                .map_err(|e| e.to_string())?;
            pending.advance(chunk)?;
            if result.added_sightings > 0 || result.updated_sightings > 0 {
                tracing::info!(
                    "backend got {} new cards and {} new sightings of {game}, and updated {} sightings",
                    result.added_cards,
                    result.added_sightings,
                    result.updated_sightings
                );
            }
        }
        Ok(())
    }
}
