use std::collections::HashMap;

use champollion_api::{Card, CardBatch, CardBatchResult, CardSighting};
use sqlx::PgPool;

use crate::error::Error;
use crate::repositories::sightings::Upsert;
use crate::repositories::{cards, sightings};

/// Most cards answered at once.
const MAX_DUE: i64 = 1000;

#[derive(Clone)]
pub struct CardService {
    db: PgPool,
}

impl CardService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Adds each card unless its lemma is known, then its sighting unless
    /// the same form was already seen on the same frame, so a batch can be
    /// sent again safely. A known sighting gets the sentence, its
    /// translation and the definition sent, so cards made again with a
    /// better prompt replace them.
    pub async fn add(&self, batch: &CardBatch) -> Result<CardBatchResult, Error> {
        let mut tx = self.db.begin().await?;
        let mut result = CardBatchResult {
            added_cards: 0,
            added_sightings: 0,
            updated_sightings: 0,
        };
        for card in &batch.cards {
            let id = match cards::insert(&mut tx, card).await? {
                Some(id) => {
                    result.added_cards += 1;
                    id
                }
                None => cards::id_of(&mut tx, card).await?,
            };
            match sightings::upsert(&mut tx, id, &card.sighting).await? {
                Upsert::Added => result.added_sightings += 1,
                Upsert::Updated => result.updated_sightings += 1,
                Upsert::Unchanged => {}
            }
        }
        tx.commit().await?;
        if result.added_sightings > 0 || result.updated_sightings > 0 {
            tracing::info!(
                "added {} cards and {} sightings, updated {} sightings, from {}",
                result.added_cards,
                result.added_sightings,
                result.updated_sightings,
                batch.cards.len()
            );
        }
        Ok(result)
    }

    /// Up to `limit` cards due now with their sightings: those already
    /// reviewed first, then new ones, each in random order. Flagged cards
    /// are left out.
    pub async fn due(&self, limit: i64) -> Result<Vec<Card>, Error> {
        let mut conn = self.db.acquire().await?;
        let due = cards::due(&mut conn, limit.clamp(1, MAX_DUE)).await?;
        let ids: Vec<i64> = due.iter().map(|c| c.id).collect();
        let mut by_card: HashMap<i64, Vec<CardSighting>> = HashMap::new();
        for s in sightings::of_cards(&mut conn, &ids).await? {
            by_card.entry(s.card_id).or_default().push(s.into());
        }
        Ok(due
            .into_iter()
            .map(|c| Card {
                sightings: by_card.remove(&c.id).unwrap_or_default(),
                id: c.id,
                lang: c.lang,
                lemma: c.lemma,
                pos: c.pos,
                gender: c.gender,
                translation: c.translation,
                due: c.due,
                reps: c.reps,
            })
            .collect())
    }
}
