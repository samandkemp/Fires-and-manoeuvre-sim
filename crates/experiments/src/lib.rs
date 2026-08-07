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

/// Value of a `--flag value` argument, parsed. `Ok(None)` if the flag is absent;
/// `Err` if it is present but its value is missing or will not parse.
///
/// The pure, testable half of [`flag_or`]. Absent and malformed are deliberately different
/// answers: the first means "take the default", the second means the caller made a mistake.
///
/// # Errors
/// A message naming the flag and what was wrong with it.
pub fn parse_flag<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>, String> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    let Some(raw) = args.get(i + 1) else {
        return Err(format!("{name} needs a value"));
    };
    raw.parse()
        .map(Some)
        .map_err(|_| format!("{name}: '{raw}' is not a valid value"))
}

/// Value of a `--flag value` argument, parsed, or `default` if the flag is absent.
///
/// **Exits the process** (status 2) if the flag is present but its value is missing or
/// unparseable, naming the flag. This used to fall back to the default instead, which meant
/// `--seeds abc` quietly ran the default 200 trials and `--until 60O` quietly ran 600 s:
/// the run succeeded and answered a different question, which is exactly the failure the
/// scenario schema's `deny_unknown_fields` exists to prevent one layer down.
///
/// Exiting rather than returning a `Result` because every caller is a `main` in this
/// crate's `src/bin/`, and the bins already handle a bad argument this way. [`parse_flag`]
/// is the pure form for anyone who wants to decide for themselves.
#[must_use]
pub fn flag_or<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    match parse_flag(args, name) {
        Ok(Some(v)) => v,
        Ok(None) => default,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Absent and malformed must be different answers. Falling back to the default on a
    /// malformed value is how `--seeds abc` used to run 200 trials in silence.
    #[test]
    fn a_missing_flag_defaults_but_a_malformed_one_is_an_error() {
        let a = args(&["--seeds", "50", "--quiet"]);
        assert_eq!(parse_flag::<u64>(&a, "--seeds"), Ok(Some(50)));
        assert_eq!(parse_flag::<u64>(&a, "--until"), Ok(None));

        let bad = args(&["--seeds", "abc"]);
        let err = parse_flag::<u64>(&bad, "--seeds").expect_err("must not silently default");
        assert!(err.contains("--seeds") && err.contains("abc"), "{err}");

        // A flag with nothing after it is a mistake too, not an absent flag.
        let dangling = args(&["--quiet", "--seeds"]);
        assert!(parse_flag::<u64>(&dangling, "--seeds").is_err());
    }
}
