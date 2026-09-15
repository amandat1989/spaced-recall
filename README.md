# spaced-recall

An SM-2 style spaced repetition scheduler: a small Rust library, plus a thin
CLI (`srs`) on top of it. No third-party dependencies.

## The problem

Spaced repetition schedulers are simple arithmetic on paper, but the input
they run on is often not trustworthy: a grade typed as `9` instead of `3`, a
card reviewed three days early because someone opened the app before the
due date, an ease factor imported from another tool's export that's already
below the floor the algorithm assumes. Feed that straight into SM-2 and you
get intervals that silently drift wrong for weeks before anyone notices.

This library treats that as a real failure mode instead of an edge case.
By default every `Scheduler` is **strict**: a malformed grade, an ease factor
under 1.3, or a review submitted before the card is due all return an error
instead of producing a card state. If you're importing messy data from
elsewhere and would rather clamp bad values than abort the batch, ask for a
**lenient** scheduler explicitly. There is no silent middle ground.

## Library usage

```rust
use spaced_recall::{Card, Scheduler};

// Day numbers are just integers your application controls - days since
// whatever epoch makes sense for you. The library has no calendar logic.
let today = 100;
let card = Card::new(today);

let scheduler = Scheduler::strict();
let after_review = scheduler.review(&card, 4, today).unwrap();

println!("next due on day {}", after_review.due_on);

// A grade outside 0-5 is a bug in the caller, not a value to guess at.
assert!(scheduler.review(&card, 9, today).is_err());

// Explicit opt-in when you'd rather clamp than fail:
let lenient = Scheduler::lenient();
let repaired = lenient.review(&card, 9, today).unwrap();
```

If you'd rather present Anki-style buttons than ask for a raw grade, use
`Rating` instead. It maps onto the same 0-5 scale, so `Hard` still counts as
a pass and `Again` still triggers a lapse:

```rust
use spaced_recall::Rating;

let after_review = scheduler.review_with_rating(&card, Rating::Good, today).unwrap();
```

## CLI usage

Create a new card, due today (day 100):

```
$ srs new --today 100
interval=0 reps=0 ease=2.50 due=100
```

Review it with a good recall (grade 4):

```
$ srs review --interval 0 --reps 0 --ease 2.50 --due 100 --today 100 --grade 4
interval=1 reps=1 ease=2.50 due=101
```

Reviewing early fails by default:

```
$ srs review --interval 6 --reps 2 --ease 2.50 --due 110 --today 105 --grade 4
error: card is not due until day 110, but today is day 105
```

Pass `--lenient` to allow it anyway:

```
$ srs review --interval 6 --reps 2 --ease 2.50 --due 110 --today 105 --grade 4 --lenient
interval=15 reps=3 ease=2.50 due=120
```

`--rating` is an alternative to `--grade` for Anki-style buttons (`again`,
`hard`, `good`, `easy`); pass one or the other, not both:

```
$ srs review --interval 0 --reps 0 --ease 2.50 --due 100 --today 100 --rating good
interval=1 reps=1 ease=2.50 due=101
```

## Deck persistence

A `Deck` is a named collection of cards, saved to a plain JSON file:

```rust
use spaced_recall::{Card, Deck};

let mut deck = Deck::new();
deck.cards.insert("capital of peru".to_string(), Card::new(100));
deck.save("deck.json").unwrap();

let loaded = Deck::load("deck.json").unwrap();
assert_eq!(loaded, deck);
```

Loading a path that doesn't exist yet returns an empty deck rather than an
error, since a deck that has never been saved isn't a malformed one. The
JSON reader and writer are hand-rolled rather than pulled in from a crate,
since the on-disk schema is fixed and small.

List the cards due on or before a given day, oldest first:

```
$ srs due --deck deck.json --today 100
capital of peru: interval=0 reps=0 ease=2.50 due=100
```

The CLI does not yet have commands to create or update named cards in a
deck file; that's the next thing to build. For now, deck files are written
by whatever is calling the library directly.

## Building

```
cargo build --release
```

There is no `Cargo.lock` committed, and none is needed: the crate has zero
dependencies.

## Algorithm

The scheduler implements the classic SuperMemo SM-2 update: two fixed
intervals for the first two successful reviews (1 day, then 6 days), then
`previous_interval * ease` after that; a lapse (grade below 3) resets the
repetition count and drops the interval back to 1 day; the ease factor is
adjusted after every review and floored at 1.3.
