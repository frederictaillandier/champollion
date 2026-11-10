use std::collections::HashMap;

use champollion_api::{Card, CardBatch, CardBatchResult, CardForm, CardSighting, DueCount};
use sqlx::PgPool;

use crate::error::Error;
use crate::repositories::sightings::Upsert;
use crate::repositories::{card_forms, cards, sightings};

/// Most cards answered at once.
const MAX_DUE: i64 = 1000;
/// A session holds one new card in this many (5 of 50), or more when too
/// few cards are due.
const NEW_EVERY: usize = 10;

#[derive(Clone)]
pub struct CardService {
    db: PgPool,
}

impl CardService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Adds each card unless its lemma is known, and its forms, replacing
    /// those it had of the same level. Then its sighting unless
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
            changed_forms: 0,
        };
        for card in &batch.cards {
            let id = match cards::insert(&mut tx, card).await? {
                Some(id) => {
                    result.added_cards += 1;
                    id
                }
                None => cards::id_of(&mut tx, card).await?,
            };
            for form in &card.forms {
                if form.level > 1 && card_forms::upsert(&mut tx, id, form).await? {
                    result.changed_forms += 1;
                }
            }
            match sightings::upsert(&mut tx, id, &card.sighting).await? {
                Upsert::Added => result.added_sightings += 1,
                Upsert::Updated => result.updated_sightings += 1,
                Upsert::Unchanged => {}
            }
        }
        tx.commit().await?;
        if result.added_sightings > 0 || result.updated_sightings > 0 || result.changed_forms > 0 {
            tracing::info!(
                "added {} cards and {} sightings, updated {} sightings and {} forms, from {}",
                result.added_cards,
                result.added_sightings,
                result.updated_sightings,
                result.changed_forms,
                batch.cards.len()
            );
        }
        Ok(result)
    }

    /// A session of up to `limit` cards with their sightings: cards due
    /// for review, in random order, then one new card in [`NEW_EVERY`], in
    /// random order. When one kind runs short, the other fills the session.
    /// Flagged cards are left out.
    pub async fn due(&self, limit: i64) -> Result<Vec<Card>, Error> {
        let limit = limit.clamp(1, MAX_DUE);
        let mut conn = self.db.acquire().await?;
        let mut reviews = cards::due_reviews(&mut conn, limit).await?;
        let mut new = cards::new_cards(&mut conn, limit).await?;
        let (review_count, new_count) = session_split(limit, reviews.len(), new.len());
        reviews.truncate(review_count);
        new.truncate(new_count);
        let due: Vec<_> = reviews
            .into_iter()
            .map(|c| (c, false))
            .chain(new.into_iter().map(|c| (c, true)))
            .collect();
        let ids: Vec<i64> = due.iter().map(|(c, _)| c.id).collect();
        let mut by_card: HashMap<i64, Vec<CardSighting>> = HashMap::new();
        for s in sightings::of_cards(&mut conn, &ids).await? {
            by_card.entry(s.card_id).or_default().push(s.into());
        }
        Ok(due
            .into_iter()
            .map(|(c, new)| Card {
                sightings: by_card.remove(&c.id).unwrap_or_default(),
                id: c.id,
                lang: c.lang,
                lemma: c.lemma,
                pos: c.pos,
                gender: c.gender,
                translation: c.translation,
                due: c.due,
                reps: c.reps,
                form: c.form.map(|form| CardForm {
                    level: c.level,
                    form,
                    translation: c.form_translation.unwrap_or_default(),
                }),
                level: c.level,
                new,
            })
            .collect())
    }

    /// How many cards are due for review now, and how many are new;
    /// flagged cards are left out.
    pub async fn due_count(&self) -> Result<DueCount, Error> {
        let mut conn = self.db.acquire().await?;
        let due = cards::due_count(&mut conn).await?;
        let new = cards::new_count(&mut conn).await?;
        Ok(DueCount {
            due: u64::try_from(due).unwrap_or(0),
            new: u64::try_from(new).unwrap_or(0),
        })
    }
}

/// How many of `reviews` cards due and `new` new cards make a session of
/// `limit`: one new card in [`NEW_EVERY`], and either kind filling what the
/// other lacks.
fn session_split(limit: i64, reviews: usize, new: usize) -> (usize, usize) {
    let limit = usize::try_from(limit).unwrap_or(0);
    let new = new.min((limit / NEW_EVERY).max(limit.saturating_sub(reviews)));
    (reviews.min(limit - new), new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_has_one_new_card_in_ten() {
        assert_eq!(session_split(50, 200, 30), (45, 5));
    }

    #[test]
    fn new_cards_fill_a_session_short_of_reviews() {
        assert_eq!(session_split(50, 12, 30), (12, 30));
        assert_eq!(session_split(50, 12, 100), (12, 38));
    }

    #[test]
    fn reviews_fill_a_session_short_of_new_cards() {
        assert_eq!(session_split(50, 200, 2), (48, 2));
        assert_eq!(session_split(50, 200, 0), (50, 0));
    }

    #[test]
    fn a_session_can_be_short_of_both() {
        assert_eq!(session_split(50, 10, 3), (10, 3));
    }
}
