//! Writing the two files every study produces.
//!
//! **Per-trial rows** (`<name>.csv`): one line per run, every metric. This is the file to
//! load into anything else — a histogram of one column answers questions a mean cannot,
//! such as whether a bimodal outcome is being averaged into a middle that never happens.
//!
//! **A summary** (`summary.csv`): one line per arm, mean and standard error for every
//! metric. Never a mean without its error bar; see [`crate::stats`].
//!
//! Deliberately hand-rolled rather than a CSV crate: every field here is a number or a
//! scenario name, so there is nothing to quote or escape, and a dependency that exists to
//! handle embedded commas would be carrying its weight for nothing.

use crate::outcome::{Outcome, COLUMNS};
use crate::stats::{tidy, Summary};
use std::fmt::Write as _;
use std::path::Path;

/// Header for a per-trial file: the key columns the caller names, then every metric.
#[must_use]
pub fn trial_header(keys: &[&str]) -> String {
    let mut s = String::new();
    for k in keys {
        let _ = write!(s, "{k},");
    }
    s.push_str(&COLUMNS.join(","));
    s.push('\n');
    s
}

/// Append one trial's row: the caller's key values, then every metric.
pub fn push_trial(out: &mut String, keys: &[String], o: &Outcome) {
    for k in keys {
        let _ = write!(out, "{k},");
    }
    let mut first = true;
    for v in o.values() {
        if !first {
            out.push(',');
        }
        let _ = write!(out, "{}", tidy(v));
        first = false;
    }
    out.push('\n');
}

/// Header for a summary file: the caller's key columns, the trial count, then a
/// `<metric>_mean,<metric>_se` pair per metric.
#[must_use]
pub fn summary_header(keys: &[&str]) -> String {
    let mut s = String::new();
    for k in keys {
        let _ = write!(s, "{k},");
    }
    s.push_str("trials");
    for c in COLUMNS {
        let _ = write!(s, ",{c}_mean,{c}_se");
    }
    s.push('\n');
    s
}

/// Append one arm's summary row, and return the per-metric summaries for the caller to
/// print or compare.
pub fn push_summary(out: &mut String, keys: &[String], outcomes: &[Outcome]) -> Vec<Summary> {
    for k in keys {
        let _ = write!(out, "{k},");
    }
    let _ = write!(out, "{}", outcomes.len());
    let summaries: Vec<Summary> = (0..COLUMNS.len())
        .map(|i| Summary::of(&crate::study::column(outcomes, i)))
        .collect();
    for s in &summaries {
        let _ = write!(out, ",{},{}", tidy(s.mean), tidy(s.se));
    }
    out.push('\n');
    summaries
}

/// Write a file, reporting failure to stderr rather than losing a completed study to a
/// `?` — the runs are the expensive part and the numbers are still on screen.
pub fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("could not create {}: {e}", parent.display());
            return;
        }
    }
    match std::fs::write(path, contents) {
        Ok(()) => println!("wrote {}", path.display()),
        Err(e) => eprintln!("could not write {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_has_one_field_per_header_column() {
        let header = trial_header(&["seed"]);
        let mut rows = String::new();
        push_trial(&mut rows, &["7".to_owned()], &Outcome::default());
        let head_fields = header.trim_end().split(',').count();
        let row_fields = rows.trim_end().split(',').count();
        assert_eq!(head_fields, row_fields, "header/row width mismatch");
        assert_eq!(head_fields, 1 + COLUMNS.len());
    }

    #[test]
    fn a_summary_row_carries_a_mean_and_an_se_per_metric() {
        let header = summary_header(&["scenario"]);
        let mut rows = String::new();
        let summaries = push_summary(
            &mut rows,
            &["default".to_owned()],
            &[Outcome::default(), Outcome::default()],
        );
        assert_eq!(summaries.len(), COLUMNS.len());
        assert_eq!(
            header.trim_end().split(',').count(),
            rows.trim_end().split(',').count()
        );
        assert_eq!(header.trim_end().split(',').count(), 2 + 2 * COLUMNS.len());
    }

    /// `-0` in a results file reads like a bug; a metric summed over an empty log is
    /// exactly how it arises.
    #[test]
    fn negative_zero_is_written_as_zero() {
        let mut rows = String::new();
        let o = Outcome {
            red_losses: -0.0,
            ..Default::default()
        };
        push_trial(&mut rows, &[], &o);
        assert!(!rows.contains("-0"), "{rows}");
    }
}
