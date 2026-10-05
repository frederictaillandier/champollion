use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Notify;

use crate::elevenlabs::{ElevenLabs, SpeakError};
use crate::error::Error;
use crate::repositories::pronunciations;

/// Time between two looks for cards without a pronunciation, when no new
/// cards wake the task up.
const ROUND: Duration = Duration::from_secs(300);
/// Cards asked from the database at once.
const BATCH: i64 = 20;

#[derive(Clone)]
pub struct PronunciationService {
    db: PgPool,
    elevenlabs: Option<Arc<ElevenLabs>>,
    wake: Arc<Notify>,
}

impl PronunciationService {
    /// Without `elevenlabs`, no pronunciation is made.
    pub fn new(db: PgPool, elevenlabs: Option<ElevenLabs>) -> Self {
        Self {
            db,
            elevenlabs: elevenlabs.map(Arc::new),
            wake: Arc::new(Notify::new()),
        }
    }

    /// The MP3 of a card's dictionary form, if it was made.
    pub async fn audio(&self, card_id: i64) -> Result<Option<Vec<u8>>, Error> {
        let mut conn = self.db.acquire().await?;
        Ok(pronunciations::audio(&mut conn, card_id).await?)
    }

    /// Makes the missing pronunciations now, not at the next round: cards
    /// were added.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Starts the task speaking the cards that have no pronunciation.
    pub fn spawn(&self) {
        let Some(elevenlabs) = self.elevenlabs.clone() else {
            tracing::info!("no ElevenLabs API key, cards are not spoken");
            return;
        };
        tracing::info!(
            "speaking cards with ElevenLabs voice {} ({})",
            elevenlabs.voice,
            elevenlabs.model
        );
        let service = self.clone();
        tokio::spawn(async move {
            // Words ElevenLabs refused, not asked again until a restart.
            let mut refused = HashSet::new();
            let mut failing = false;
            loop {
                match service.speak_missing(&elevenlabs, &mut refused).await {
                    Ok(made) => {
                        if failing {
                            tracing::info!("speaking cards works again");
                            failing = false;
                        }
                        if made > 0 {
                            tracing::info!("spoke {made} cards");
                        }
                    }
                    Err(e) => {
                        if !failing {
                            tracing::warn!("could not speak cards, will retry: {e}");
                        }
                        failing = true;
                    }
                }
                continue_after(&service.wake).await;
            }
        });
    }

    /// Speaks every card without a pronunciation; returns how many.
    async fn speak_missing(
        &self,
        elevenlabs: &Arc<ElevenLabs>,
        refused: &mut HashSet<i64>,
    ) -> Result<usize, String> {
        let mut made = 0;
        loop {
            let mut conn = self.db.acquire().await.map_err(|e| e.to_string())?;
            let skip: Vec<i64> = refused.iter().copied().collect();
            let cards = pronunciations::missing(&mut conn, &skip, BATCH)
                .await
                .map_err(|e| e.to_string())?;
            if cards.is_empty() {
                return Ok(made);
            }
            for card in cards {
                let client = Arc::clone(elevenlabs);
                let (lemma, lang) = (card.lemma.clone(), card.lang.clone());
                let spoken = tokio::task::spawn_blocking(move || client.speak(&lemma, &lang))
                    .await
                    .map_err(|e| e.to_string())?;
                match spoken {
                    Ok(audio) => {
                        pronunciations::upsert(
                            &mut conn,
                            card.id,
                            &audio,
                            &elevenlabs.voice,
                            &elevenlabs.model,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        made += 1;
                    }
                    Err(SpeakError::Text(e)) => {
                        tracing::warn!("could not speak {:?}: {e}", card.lemma);
                        refused.insert(card.id);
                    }
                    Err(SpeakError::Service(e)) => return Err(e),
                }
            }
        }
    }
}

/// Waits for the next round, or for new cards.
async fn continue_after(wake: &Notify) {
    tokio::select! {
        _ = wake.notified() => {}
        _ = tokio::time::sleep(ROUND) => {}
    }
}
