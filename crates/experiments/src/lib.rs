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

/// Value of a `--flag value` argument. `Ok(None)` if the flag is absent; `Err` if it is
/// present but its value is missing or is itself a flag.
///
/// The pure, testable half of [`flag`], and the string counterpart of [`parse_flag`].
///
/// # Errors
/// A message naming the flag and what was wrong with it.
pub fn try_flag(args: &[String], name: &str) -> Result<Option<String>, String> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    match args.get(i + 1) {
        None => Err(format!("{name} needs a value")),
        // A `--`-prefixed value is the next flag, reached because this one's value was
        // dropped. Single-dash values are left alone: `--from -1.0` is legitimate.
        Some(v) if v.starts_with("--") => Err(format!("{name} needs a value, got '{v}'")),
        Some(v) => Ok(Some(v.clone())),
    }
}

/// Value of a `--flag value` argument, or `None` if the flag is absent.
///
/// **Exits the process** (status 2) if the flag is present but its value is missing or is
/// itself a flag, for exactly the reason [`flag_or`] does. Nearly every caller pairs this
/// with a default, so returning `None` for a dangling `--metric` would read as "no metric
/// asked for", quietly measure `red_losses` instead, and report a study that answers a
/// different question. [`try_flag`] is the pure form.
#[must_use]
pub fn flag(args: &[String], name: &str) -> Option<String> {
    or_exit(try_flag(args, name))
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
    or_exit(parse_flag(args, name)).unwrap_or(default)
}

/// Every `--flag value` occurrence of a repeatable argument, in order. `Err` on an
/// occurrence whose value is missing or is itself a flag.
///
/// The pure, testable half of [`flags`].
///
/// # Errors
/// A message naming the flag and what was wrong with it.
pub fn try_flags(args: &[String], name: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] != name {
            i += 1;
            continue;
        }
        match args.get(i + 1) {
            Some(v) if !v.starts_with("--") => out.push(v.clone()),
            None => return Err(format!("{name} needs a value")),
            Some(v) => return Err(format!("{name} needs a value, got '{v}'")),
        }
        // Step over the value, so one that happens to equal `name` is not read as a
        // further occurrence of the flag.
        i += 2;
    }
    Ok(out)
}

/// Every `--flag value` occurrence of a repeatable argument, in order.
///
/// **Exits the process** (status 2) on an occurrence whose value is missing or is itself a
/// flag, for the reason [`flag`] does. [`try_flags`] is the pure form.
#[must_use]
pub fn flags(args: &[String], name: &str) -> Vec<String> {
    or_exit(try_flags(args, name))
}

/// Report a bad argument and stop, or hand back the value.
///
/// Every caller in this crate is a `main` in `src/bin/`, and they all answer a malformed
/// argument the same way: say which flag was wrong and exit 2.
fn or_exit<T>(parsed: Result<T, String>) -> T {
    match parsed {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
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

    /// The same contract for string flags. A dangling `--metric` must not read as "no
    /// metric asked for": every caller pairs it with a default, so that would silently
    /// measure the wrong column and still report a clean study.
    #[test]
    fn a_dangling_string_flag_is_an_error_not_an_absent_one() {
        let ok = args(&["--metric", "red_losses", "--quiet"]);
        assert_eq!(try_flag(&ok, "--metric"), Ok(Some("red_losses".to_owned())));
        assert_eq!(try_flag(&ok, "--only"), Ok(None));

        let dangling = args(&["--quiet", "--metric"]);
        assert!(try_flag(&dangling, "--metric").is_err());

        // The value was dropped and the next flag slid into its place.
        let swallowed = args(&["--metric", "--seeds", "500"]);
        let err = try_flag(&swallowed, "--metric").expect_err("must not accept a flag as a value");
        assert!(err.contains("--metric") && err.contains("--seeds"), "{err}");

        // A negative number is a value, not a flag.
        let negative = args(&["--from", "-1.0"]);
        assert_eq!(try_flag(&negative, "--from"), Ok(Some("-1.0".to_owned())));
    }

    /// Repeatable flags collect in order, and a value equal to the flag's own name is a
    /// value rather than a further occurrence of it.
    #[test]
    fn repeatable_flags_collect_in_order_and_validate_each() {
        let a = args(&["--set", "a=1", "--quiet", "--set", "b=2"]);
        assert_eq!(
            try_flags(&a, "--set"),
            Ok(vec!["a=1".to_owned(), "b=2".to_owned()])
        );
        assert_eq!(try_flags(&a, "--factor"), Ok(Vec::new()));

        let dangling = args(&["--set", "a=1", "--set"]);
        assert!(try_flags(&dangling, "--set").is_err());

        let swallowed = args(&["--set", "--seeds"]);
        assert!(try_flags(&swallowed, "--set").is_err());

        // `--set --set` would double-count if the value were not stepped over.
        let literal = args(&["--set", "-set"]);
        assert_eq!(try_flags(&literal, "--set"), Ok(vec!["-set".to_owned()]));
    }
}
