//! When to show a card again: SM-2, the algorithm Anki started from, with
//! two ratings and gaps starting at minutes rather than days. A card learnt
//! well enough moves up to the level of its next form, which is learnt from
//! the start again; it never moves down.

use champollion_api::Rating;
use chrono::{DateTime, Duration, Utc};

/// The shortest gap: after a failure, or the first success.
const MIN_GAP_DAYS: f64 = 10.0 / (24.0 * 60.0);
/// Each success multiplies the gap by the card's ease and this.
const SUCCESS_BONUS: f64 = 1.3;
const MIN_EASE: f64 = 1.3;
/// The ease of a new card, and of a card moving up a level.
const START_EASE: f64 = 2.5;
/// A success making the gap longer than this moves the card up a level.
const LEVEL_UP_GAP_DAYS: f64 = 7.0;

#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    pub interval_days: f64,
    pub ease: f64,
    /// Successful reviews in a row.
    pub reps: i32,
    pub lapses: i32,
    pub level: i16,
}

impl CardState {
    /// The card's state after a review, and when it is due next.
    /// `next_level`: whether the card has a form for the level above.
    pub fn review(
        &self,
        rating: Rating,
        at: DateTime<Utc>,
        next_level: bool,
    ) -> (CardState, DateTime<Utc>) {
        let mut next = self.clone();
        match rating {
            Rating::Failed => {
                next.reps = 0;
                next.lapses += 1;
                next.ease = (self.ease - 0.2).max(MIN_EASE);
                next.interval_days = MIN_GAP_DAYS;
            }
            Rating::Succeeded => {
                next.reps += 1;
                next.ease = self.ease + 0.15;
                next.interval_days = match self.reps {
                    0 => MIN_GAP_DAYS,
                    _ => (self.interval_days * self.ease * SUCCESS_BONUS).max(MIN_GAP_DAYS),
                };
                if next_level && next.interval_days > LEVEL_UP_GAP_DAYS {
                    next.level += 1;
                    next.ease = START_EASE;
                    next.interval_days = MIN_GAP_DAYS;
                }
            }
        }
        let due = at + Duration::seconds((next.interval_days * 86_400.0).round() as i64);
        (next, due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_card() -> CardState {
        CardState {
            interval_days: 0.0,
            ease: START_EASE,
            reps: 0,
            lapses: 0,
            level: 1,
        }
    }

    fn at() -> DateTime<Utc> {
        "2026-10-04T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn successes_space_out_reviews_from_ten_minutes() {
        let (card, due) = new_card().review(Rating::Succeeded, at(), false);
        assert_eq!(due, at() + Duration::minutes(10));
        // 10 min × 2.65 × 1.3, the ease having grown by 0.15
        let (card, due) = card.review(Rating::Succeeded, at(), false);
        assert_eq!(due, at() + Duration::seconds(2067));
        // 34.45 min × 2.8 × 1.3
        let (card, due) = card.review(Rating::Succeeded, at(), false);
        assert_eq!(due, at() + Duration::seconds(7524));
        assert_eq!(card.reps, 3);
    }

    #[test]
    fn a_failure_brings_the_gap_back_to_ten_minutes() {
        let mut card = new_card();
        for _ in 0..8 {
            card = card.review(Rating::Succeeded, at(), false).0;
        }
        assert!(card.interval_days > 1.0);
        let (card, due) = card.review(Rating::Failed, at(), false);
        assert_eq!(due, at() + Duration::minutes(10));
        assert_eq!((card.reps, card.lapses), (0, 1));
        let (_, due) = card.review(Rating::Succeeded, at(), false);
        assert_eq!(due, at() + Duration::minutes(10));
    }

    #[test]
    fn failures_lower_the_ease_down_to_the_minimum() {
        let (card, _) = new_card().review(Rating::Failed, at(), false);
        assert_eq!(card.ease, 2.3);
        let mut card = card;
        for _ in 0..20 {
            card = card.review(Rating::Failed, at(), false).0;
        }
        assert_eq!(card.ease, MIN_EASE);
    }

    #[test]
    fn a_gap_over_a_week_moves_the_card_up_a_level_from_ten_minutes() {
        let mut card = new_card();
        while card.interval_days * card.ease * SUCCESS_BONUS <= LEVEL_UP_GAP_DAYS {
            card = card.review(Rating::Succeeded, at(), true).0;
        }
        assert_eq!(card.level, 1);
        let reps = card.reps;
        let (card, due) = card.review(Rating::Succeeded, at(), true);
        assert_eq!(due, at() + Duration::minutes(10));
        assert_eq!(
            (card.level, card.ease, card.reps),
            (2, START_EASE, reps + 1)
        );
        // 10 min × 2.5 × 1.3: from 10 minutes, not from the second gap.
        let (_, due) = card.review(Rating::Succeeded, at(), true);
        assert_eq!(due, at() + Duration::seconds(1950));
    }

    #[test]
    fn a_card_without_a_next_form_stays_at_its_level() {
        let mut card = new_card();
        for _ in 0..10 {
            card = card.review(Rating::Succeeded, at(), false).0;
        }
        assert!(card.interval_days > LEVEL_UP_GAP_DAYS);
        assert_eq!(card.level, 1);
    }

    #[test]
    fn failures_never_move_a_card_down() {
        let mut card = CardState {
            level: 2,
            ..new_card()
        };
        for _ in 0..20 {
            card = card.review(Rating::Failed, at(), true).0;
        }
        assert_eq!(card.level, 2);
    }

    #[test]
    fn ratings_from_older_phone_builds_are_still_read() {
        let read = |name: &str| serde_json::from_str::<Rating>(&format!("\"{name}\"")).unwrap();
        assert_eq!(read("again"), Rating::Failed);
        assert_eq!(read("easy"), Rating::Succeeded);
        assert_eq!(read("succeeded"), Rating::Succeeded);
    }
}
