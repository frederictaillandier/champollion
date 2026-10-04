//! When to show a card again: SM-2, the algorithm Anki started from.

use champollion_api::Rating;
use chrono::{DateTime, Duration, Utc};

/// A card forgotten again is shown after this delay, in the same session.
const RELEARN_DELAY: Duration = Duration::minutes(10);
const MIN_EASE: f64 = 1.3;

#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    pub interval_days: f64,
    pub ease: f64,
    /// Successful reviews in a row.
    pub reps: i32,
    pub lapses: i32,
}

impl CardState {
    /// The card's state after a review, and when it is due next.
    pub fn review(&self, rating: Rating, at: DateTime<Utc>) -> (CardState, DateTime<Utc>) {
        let mut next = self.clone();
        let interval = match rating {
            Rating::Again => {
                next.reps = 0;
                next.lapses += 1;
                next.ease = (self.ease - 0.2).max(MIN_EASE);
                next.interval_days = 0.0;
                return (next, at + RELEARN_DELAY);
            }
            Rating::Hard => {
                next.ease = (self.ease - 0.15).max(MIN_EASE);
                match self.reps {
                    0 => 1.0,
                    _ => (self.interval_days * 1.2).max(1.0),
                }
            }
            Rating::Good => match self.reps {
                0 => 1.0,
                1 => 6.0,
                _ => self.interval_days * self.ease,
            },
            Rating::Easy => {
                next.ease = self.ease + 0.15;
                match self.reps {
                    0 => 4.0,
                    _ => (self.interval_days * self.ease * 1.3).max(6.0),
                }
            }
        };
        next.reps += 1;
        next.interval_days = interval;
        let due = at + Duration::seconds((interval * 86_400.0) as i64);
        (next, due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_card() -> CardState {
        CardState {
            interval_days: 0.0,
            ease: 2.5,
            reps: 0,
            lapses: 0,
        }
    }

    fn at() -> DateTime<Utc> {
        "2026-10-04T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn good_answers_space_out_reviews() {
        let (card, due) = new_card().review(Rating::Good, at());
        assert_eq!(due, at() + Duration::days(1));
        let (card, due) = card.review(Rating::Good, at());
        assert_eq!(due, at() + Duration::days(6));
        let (card, due) = card.review(Rating::Good, at());
        assert_eq!(due, at() + Duration::days(15));
        assert_eq!(card.reps, 3);
    }

    #[test]
    fn again_resets_the_card_and_lowers_its_ease() {
        let (card, _) = new_card().review(Rating::Good, at());
        let (card, _) = card.review(Rating::Good, at());
        let (card, due) = card.review(Rating::Again, at());
        assert_eq!(due, at() + RELEARN_DELAY);
        assert_eq!((card.reps, card.lapses, card.ease), (0, 1, 2.3));
    }

    #[test]
    fn ease_never_drops_below_the_minimum() {
        let mut card = new_card();
        for _ in 0..20 {
            card = card.review(Rating::Again, at()).0;
        }
        assert_eq!(card.ease, MIN_EASE);
    }

    #[test]
    fn easy_goes_further_than_good() {
        let (_, good) = new_card().review(Rating::Good, at());
        let (_, easy) = new_card().review(Rating::Easy, at());
        assert!(easy > good);
    }
}
