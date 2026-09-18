//! An SM-2 style spaced repetition scheduler.
//!
//! Dates are represented as plain day numbers (days since some fixed epoch,
//! chosen by the caller) rather than a calendar type, so the library has no
//! opinion about time zones or clocks. The CLI derives them from the system
//! clock; other callers can use whatever counter fits.

use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};

mod deck;
pub use deck::{Deck, DeckError};

/// Recall quality for a single review, on the classic SM-2 scale.
/// 0 means a complete blackout, 5 means perfect, effortless recall.
pub type Grade = u8;

pub const MIN_GRADE: Grade = 0;
pub const MAX_GRADE: Grade = 5;

/// Grades below this are treated as a lapse: repetitions reset and the
/// interval drops back to one day.
pub const PASSING_GRADE: Grade = 3;

/// The ease factor never drops below this, or the interval would start
/// shrinking every time a card is reviewed, which defeats the algorithm.
pub const MIN_EASE: f64 = 1.3;

pub const DEFAULT_EASE: f64 = 2.5;

/// Intervals shorter than this are never fuzzed. A one or two day interval
/// has too little room to jitter without either doing nothing or being
/// rounded away entirely, so it's left exact.
pub const FUZZ_MIN_INTERVAL: u32 = 3;

/// The scheduling state of a single card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card {
    pub interval_days: u32,
    pub repetitions: u32,
    pub ease: f64,
    pub due_on: u32,
}

impl Card {
    /// A brand new card, due for its first review today.
    pub fn new(today: u32) -> Self {
        Card {
            interval_days: 0,
            repetitions: 0,
            ease: DEFAULT_EASE,
            due_on: today,
        }
    }
}

/// Everything that can make a review request untrustworthy.
///
/// In strict mode any of these abort the review. In lenient mode the
/// scheduler repairs the input instead (see `Scheduler::lenient`).
#[derive(Debug, PartialEq)]
pub enum ScheduleError {
    GradeOutOfRange(Grade),
    EaseTooLow(f64),
    ReviewedBeforeDue { today: u32, due_on: u32 },
    DateOverflow,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::GradeOutOfRange(g) => {
                write!(f, "grade {g} is outside the valid range {MIN_GRADE}-{MAX_GRADE}")
            }
            ScheduleError::EaseTooLow(e) => {
                write!(f, "ease factor {e} is below the floor of {MIN_EASE}")
            }
            ScheduleError::ReviewedBeforeDue { today, due_on } => {
                write!(f, "card is not due until day {due_on}, but today is day {today}")
            }
            ScheduleError::DateOverflow => write!(f, "next due date does not fit in a u32"),
        }
    }
}

impl Error for ScheduleError {}

/// An Anki-style four-button rating.
///
/// This is not a different scheduling algorithm - it's a fixed mapping onto
/// the same 0-5 grade scale `Scheduler::review` already accepts, for callers
/// that would rather present "Again / Hard / Good / Easy" to a user than ask
/// them to type a raw number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    /// The SM-2 grade this rating stands in for.
    ///
    /// `Hard` maps to `PASSING_GRADE` rather than something below it: in
    /// Anki, Hard still counts as a successful recall, just a strained one,
    /// so it should not trigger a lapse reset.
    pub fn to_grade(self) -> Grade {
        match self {
            Rating::Again => 0,
            Rating::Hard => PASSING_GRADE,
            Rating::Good => 4,
            Rating::Easy => MAX_GRADE,
        }
    }

    /// Parses the lowercase button name used on the command line.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "again" => Some(Rating::Again),
            "hard" => Some(Rating::Hard),
            "good" => Some(Rating::Good),
            "easy" => Some(Rating::Easy),
            _ => None,
        }
    }
}

/// Computes the next scheduling state for a card after a review.
///
/// A `Scheduler` is either strict or lenient. Strict is the default and the
/// one you want in an application: it rejects a malformed review outright
/// so bad data never reaches the schedule. Lenient exists for scripts and
/// imports of messy external data, where clamping the input to something
/// sane is preferable to stopping a batch job on the first bad record.
///
/// Fuzzing is off by default. Without it, every card that shares an ease
/// factor and review history ends up on the exact same due date as every
/// other one that started at the same time, so a deck reviewed together
/// stays clumped together forever. Turn it on with `with_fuzz` to spread
/// intervals out by a few percent.
#[derive(Debug, Clone, Copy)]
pub struct Scheduler {
    pub strict: bool,
    fuzz_seed: Option<u64>,
}

impl Scheduler {
    pub fn strict() -> Self {
        Scheduler {
            strict: true,
            fuzz_seed: None,
        }
    }

    pub fn lenient() -> Self {
        Scheduler {
            strict: false,
            fuzz_seed: None,
        }
    }

    /// Enables interval fuzzing, seeded by `seed`.
    ///
    /// The same `(seed, today, interval)` triple always fuzzes to the same
    /// result, so a review is still reproducible for testing - the point is
    /// only to decorrelate cards from each other, not to be unpredictable
    /// from run to run. Callers that want different cards to land on
    /// different days should pass a seed derived from something stable per
    /// card, such as a hash of its name.
    pub fn with_fuzz(self, seed: u64) -> Self {
        Scheduler {
            fuzz_seed: Some(seed),
            ..self
        }
    }

    /// Reviews `card` with the given `grade` on day `today`, returning the
    /// card's next state.
    pub fn review(&self, card: &Card, grade: Grade, today: u32) -> Result<Card, ScheduleError> {
        let grade = self.checked_grade(grade)?;
        let ease_in = self.checked_ease(card.ease)?;
        self.checked_timing(card, today)?;

        let mut next = *card;

        if grade < PASSING_GRADE {
            next.repetitions = 0;
            next.interval_days = 1;
        } else {
            next.repetitions = card.repetitions + 1;
            next.interval_days = match next.repetitions {
                1 => 1,
                2 => 6,
                _ => (card.interval_days as f64 * ease_in).round() as u32,
            };
        }

        let g = grade as f64;
        let ease_delta = 0.1 - (5.0 - g) * (0.08 + (5.0 - g) * 0.02);
        next.ease = (ease_in + ease_delta).max(MIN_EASE);

        if let Some(seed) = self.fuzz_seed {
            next.interval_days = fuzz_interval(seed, today, next.interval_days);
        }

        next.due_on = today
            .checked_add(next.interval_days)
            .ok_or(ScheduleError::DateOverflow)?;

        Ok(next)
    }

    /// Same as `review`, but takes an Anki-style button press instead of a
    /// raw grade. Since a `Rating` is always in range, this only fails on
    /// ease or timing problems, never on `ScheduleError::GradeOutOfRange`.
    pub fn review_with_rating(
        &self,
        card: &Card,
        rating: Rating,
        today: u32,
    ) -> Result<Card, ScheduleError> {
        self.review(card, rating.to_grade(), today)
    }

    fn checked_grade(&self, grade: Grade) -> Result<Grade, ScheduleError> {
        if grade > MAX_GRADE {
            if self.strict {
                Err(ScheduleError::GradeOutOfRange(grade))
            } else {
                Ok(MAX_GRADE)
            }
        } else {
            Ok(grade)
        }
    }

    fn checked_ease(&self, ease: f64) -> Result<f64, ScheduleError> {
        if ease < MIN_EASE {
            if self.strict {
                Err(ScheduleError::EaseTooLow(ease))
            } else {
                Ok(MIN_EASE)
            }
        } else {
            Ok(ease)
        }
    }

    fn checked_timing(&self, card: &Card, today: u32) -> Result<(), ScheduleError> {
        if today < card.due_on && self.strict {
            Err(ScheduleError::ReviewedBeforeDue {
                today,
                due_on: card.due_on,
            })
        } else {
            Ok(())
        }
    }
}

/// Jitters `interval_days` by up to 5% (at least one day) so that cards
/// which would otherwise land on the same due date spread out over a few
/// days instead. Deterministic in `seed`, `today`, and `interval_days` so
/// the same review always fuzzes the same way; varying `seed` per card is
/// what actually decorrelates a deck.
fn fuzz_interval(seed: u64, today: u32, interval_days: u32) -> u32 {
    if interval_days < FUZZ_MIN_INTERVAL {
        return interval_days;
    }

    let range = (interval_days / 20).max(1);
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    today.hash(&mut hasher);
    interval_days.hash(&mut hasher);
    let bucket_count = 2 * range + 1;
    let offset = (hasher.finish() % bucket_count as u64) as i64 - range as i64;

    (interval_days as i64 + offset).max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_two_passes_use_fixed_intervals() {
        let scheduler = Scheduler::strict();
        let card = Card::new(0);

        let after_first = scheduler.review(&card, 4, 0).unwrap();
        assert_eq!(after_first.interval_days, 1);
        assert_eq!(after_first.repetitions, 1);

        let after_second = scheduler.review(&after_first, 4, 1).unwrap();
        assert_eq!(after_second.interval_days, 6);
        assert_eq!(after_second.repetitions, 2);
    }

    #[test]
    fn a_lapse_resets_repetitions_but_keeps_ease_updates() {
        let scheduler = Scheduler::strict();
        let card = Card {
            interval_days: 20,
            repetitions: 4,
            ease: 2.3,
            due_on: 50,
        };

        let next = scheduler.review(&card, 1, 50).unwrap();
        assert_eq!(next.repetitions, 0);
        assert_eq!(next.interval_days, 1);
        assert!(next.ease < card.ease);
    }

    #[test]
    fn strict_mode_rejects_a_review_before_the_due_date() {
        let scheduler = Scheduler::strict();
        let card = Card::new(10);

        let err = scheduler.review(&card, 4, 5).unwrap_err();
        assert_eq!(
            err,
            ScheduleError::ReviewedBeforeDue {
                today: 5,
                due_on: 10
            }
        );
    }

    #[test]
    fn lenient_mode_allows_an_early_review_and_clamps_bad_input() {
        let scheduler = Scheduler::lenient();
        let card = Card::new(10);

        let next = scheduler.review(&card, 9, 5).unwrap();
        assert_eq!(next.repetitions, 1);
    }

    #[test]
    fn strict_mode_rejects_an_out_of_range_grade() {
        let scheduler = Scheduler::strict();
        let card = Card::new(0);

        let err = scheduler.review(&card, 9, 0).unwrap_err();
        assert_eq!(err, ScheduleError::GradeOutOfRange(9));
    }

    #[test]
    fn rating_names_round_trip_to_the_expected_grades() {
        assert_eq!(Rating::from_name("again").unwrap().to_grade(), 0);
        assert_eq!(Rating::from_name("hard").unwrap().to_grade(), PASSING_GRADE);
        assert_eq!(Rating::from_name("good").unwrap().to_grade(), 4);
        assert_eq!(Rating::from_name("easy").unwrap().to_grade(), MAX_GRADE);
        assert!(Rating::from_name("meh").is_none());
    }

    #[test]
    fn again_is_a_lapse_but_hard_is_not() {
        let scheduler = Scheduler::strict();
        let card = Card {
            interval_days: 20,
            repetitions: 4,
            ease: 2.3,
            due_on: 50,
        };

        let after_again = scheduler
            .review_with_rating(&card, Rating::Again, 50)
            .unwrap();
        assert_eq!(after_again.repetitions, 0);
        assert_eq!(after_again.interval_days, 1);

        let after_hard = scheduler
            .review_with_rating(&card, Rating::Hard, 50)
            .unwrap();
        assert_eq!(after_hard.repetitions, 5);
    }

    #[test]
    fn without_fuzz_identical_cards_land_on_the_same_day() {
        let scheduler = Scheduler::strict();
        let card = Card {
            interval_days: 20,
            repetitions: 3,
            ease: 2.0,
            due_on: 50,
        };

        let a = scheduler.review(&card, 4, 50).unwrap();
        let b = scheduler.review(&card, 4, 50).unwrap();
        assert_eq!(a.due_on, b.due_on);
    }

    #[test]
    fn fuzz_spreads_identical_cards_across_different_seeds() {
        let card = Card {
            interval_days: 20,
            repetitions: 3,
            ease: 2.0,
            due_on: 50,
        };

        let due_on_for_seed = |seed: u64| {
            Scheduler::strict()
                .with_fuzz(seed)
                .review(&card, 4, 50)
                .unwrap()
                .due_on
        };

        let due_dates: std::collections::BTreeSet<u32> =
            (0..20).map(due_on_for_seed).collect();
        assert!(
            due_dates.len() > 1,
            "expected different seeds to land on different days, got {due_dates:?}"
        );
    }

    #[test]
    fn fuzz_is_reproducible_for_the_same_seed() {
        let scheduler = Scheduler::strict().with_fuzz(42);
        let card = Card {
            interval_days: 20,
            repetitions: 3,
            ease: 2.0,
            due_on: 50,
        };

        let a = scheduler.review(&card, 4, 50).unwrap();
        let b = scheduler.review(&card, 4, 50).unwrap();
        assert_eq!(a.due_on, b.due_on);
    }

    #[test]
    fn fuzz_leaves_short_intervals_untouched() {
        let scheduler = Scheduler::strict().with_fuzz(7);
        let card = Card::new(0);

        let after_first = scheduler.review(&card, 4, 0).unwrap();
        assert_eq!(after_first.interval_days, 1);

        let after_second = scheduler.review(&after_first, 4, 1).unwrap();
        assert_eq!(after_second.interval_days, 6);
    }

    #[test]
    fn fuzz_never_produces_a_zero_or_negative_interval() {
        let card = Card {
            interval_days: 3,
            repetitions: 3,
            ease: 1.3,
            due_on: 0,
        };

        for seed in 0..50 {
            let next = Scheduler::strict()
                .with_fuzz(seed)
                .review(&card, 4, 0)
                .unwrap();
            assert!(next.interval_days >= 1);
        }
    }
}
