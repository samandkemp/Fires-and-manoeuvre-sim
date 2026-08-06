//! The batch harness: run a scenario many thousands of times and report what happened,
//! with error bars.
//!
//! The app shows one battle. A study needs the distribution — a mean is worthless without
//! knowing whether the difference you are looking at is bigger than the noise. This module
//! is the shared machinery every headless binary in this crate draws on, so a new
//! experiment is a CLI and a question, not another copy of the plumbing.
//!
//! # Where things live
//!
//! | Module | What it does |
//! |---|---|
//! | [`outcome`] | what one run produced, and the column order it writes as |
//! | [`study`] | running N seeds in parallel, with progress |
//! | [`stats`] | means, standard errors, and **paired** differences |
//! | [`patch`] | overriding a dial in a scenario's TOML before it is parsed |
//! | [`csv`] | writing the two files every study produces |
//!
//! # The two rules this harness exists to enforce
//!
//! **Fix the map, vary the dice.** [`Sim::new`] derives the terrain *and* the RNG stream
//! from one seed, so looping it over seeds varies both at once and averages two sources of
//! variance together. Every study here builds terrain once per worker at the scenario's
//! own seed and calls [`Sim::reset_to_scenario`] per trial, so the map is held fixed and
//! the question is "what happens on *this* map, on average".
//!
//! **Compare paired, always.** Two arms of a study run the *same seed set*, so the
//! difference between them can be taken seed by seed. This is not a nicety: an unpaired
//! comparison of the fire-allocation solvers once produced a confident, entirely spurious
//! finding that greedy beat the optimal assignment (`docs/DESIGN.md` §10.2). Common random
//! numbers cancel the map-and-dice variance the two arms share, which is usually most of
//! it, and [`stats::paired`] is the only comparison function this crate offers.

pub mod csv;
pub mod outcome;
pub mod patch;
pub mod stats;
pub mod study;

pub use outcome::{Outcome, COLUMNS};
pub use stats::{mean_and_se, paired, Paired, Summary};
pub use study::{run_study, StudyConfig};

/// Value of a `--flag value` argument.
#[must_use]
pub fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

/// Value of a `--flag value` argument, parsed, or `default` if absent or unparseable.
#[must_use]
pub fn flag_or<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    flag(args, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Every `--flag value` occurrence of a repeatable argument, in order.
#[must_use]
pub fn flags(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].clone())
        .collect()
}

/// Is a bare `--flag` present?
#[must_use]
pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
