//! The batch harness: run a scenario many thousands of times and report what happened,
//! with error bars.
//!
//! The shared machinery every headless binary draws on, so a new experiment is a CLI and a
//! question rather than another copy of the plumbing.
//!
//! # Where things live
//!
//! | Module | What it does |
//! |---|---|
//! | [`metrics`] | what one run produced, and the column order it writes as |
//! | [`study`] | running N seeds of one arm in parallel |
//! | [`factorial_design`] | factorial designs: several dials at once, and their interactions |
//! | [`sensitivity`] | Morris screening and Sobol decomposition over a dial space |
//! | [`stats`] | means, standard errors, and **paired** differences |
//! | [`overrides`] | overriding a dial in a scenario's TOML before it is parsed |
//! | [`dials`] | the registry of every dial and a sensible range for it |
//! | [`findings`] | re-running the numbers the documentation states |
//! | [`experiment`] | an experiment as a value that can be queued, saved and re-run |
//! | [`csv`] | writing the two files every study produces |
//!
//! # The two rules this harness enforces (`docs/THEORY.md` §14.1-§14.2)
//!
//! **Fix the map, vary the dice.** `Sim::new` derives terrain *and* the RNG stream from
//! one seed, so looping it over seeds averages two sources of variance together. Every
//! study builds terrain once per worker at the scenario's own seed and calls
//! `Sim::reset_to_scenario` per trial, holding the map fixed.
//!
//! **Compare paired, always.** [`stats::paired`] is the only comparison this crate offers.

pub mod csv;
pub mod dials;
pub mod experiment;
pub mod factorial_design;
pub mod findings;
pub mod metrics;
pub mod overrides;
pub mod sensitivity;
pub mod stats;
pub mod study;

pub use metrics::{Outcome, COLUMNS};
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
/// The pure, testable half of [`flag_or`]. Absent and malformed are different
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
/// unparseable, naming the flag. Falling back to the default would let `--seeds abc` run
/// 200 trials and `--until 60O` run 600 s - the run succeeds and answers a different
/// question, which is the failure `deny_unknown_fields` prevents one layer down.
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

    /// Absent and malformed must be different answers: falling back to the default on a
    /// malformed value would let `--seeds abc` run 200 trials in silence.
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
