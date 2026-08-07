//! Writing the two files every study produces.
//!
//! **Per-trial rows** (`<name>.csv`): one line per run, every metric. This is the file to
//! load into anything else — a histogram of one column answers questions a mean cannot,
//! such as whether a bimodal outcome is being averaged into a middle that never happens.
//!
//! **A summary** (`summary.csv`): one line per arm, mean and standard error for every
//! metric. Never a mean without its error bar; see [`crate::stats`].
//!
//! Deliberately hand-rolled rather than a CSV crate: the metric columns are all numbers,
//! and the only fields that can hold anything else are the caller's **key** columns, which
//! [`field`] quotes when they need it.
//!
//! Those keys did once need nothing. `sweep` then began passing the swept *value* as a key,
//! and its documented usage includes list-valued dials — `--values '["c2","air_defence"]'`
//! — which carry commas. An unquoted row for that arm has extra fields and silently
//! misaligns every column after it, which is worse than failing: the file still loads.

use crate::outcome::{Outcome, COLUMNS};
use crate::stats::{tidy, Summary};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::Path;

/// One field, quoted per RFC 4180 **only if it needs to be**.
///
/// Quoting unconditionally would be simpler but would rewrite every existing results file
/// (`0` becoming `"0"`), so the common case is left byte-identical and only a field holding
/// a comma, a quote or a newline is wrapped — with any interior quote doubled.
fn field(raw: &str) -> Cow<'_, str> {
    if !raw.contains([',', '"', '\n', '\r']) {
        return Cow::Borrowed(raw);
    }
    Cow::Owned(format!("\"{}\"", raw.replace('"', "\"\"")))
}

/// Header for a per-trial file: the key columns the caller names, then every metric.
#[must_use]
pub fn trial_header(keys: &[&str]) -> String {
    let mut s = String::new();
    for k in keys {
        let _ = write!(s, "{},", field(k));
    }
    s.push_str(&COLUMNS.join(","));
    s.push('\n');
    s
}

/// Append one trial's row: the caller's key values, then every metric.
pub fn push_trial(out: &mut String, keys: &[String], o: &Outcome) {
    for k in keys {
        let _ = write!(out, "{},", field(k));
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
        let _ = write!(s, "{},", field(k));
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
        let _ = write!(out, "{},", field(k));
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

    /// A key holding a comma — which `sweep` produces for a list-valued dial — must not
    /// silently add a field and misalign every column after it.
    #[test]
    fn a_key_containing_a_comma_is_quoted_and_keeps_the_row_aligned() {
        let header = trial_header(&["value", "seed"]);
        let mut rows = String::new();
        push_trial(
            &mut rows,
            &[r#"["c2", "air_defence"]"#.to_owned(), "3".to_owned()],
            &Outcome::default(),
        );
        assert!(rows.starts_with('"'), "the key must be quoted: {rows}");
        assert!(rows.contains(r#"""c2"", ""air_defence"""#), "{rows}");
        assert_eq!(
            fields(&rows),
            header.trim_end().split(',').count(),
            "quoting must keep the row the same width as the header"
        );
        // An ordinary key is left exactly as it was, so existing files do not change.
        let mut plain = String::new();
        push_trial(&mut plain, &["7".to_owned()], &Outcome::default());
        assert!(plain.starts_with("7,"), "{plain}");
    }

    /// Count fields the way a CSV reader does: a comma inside quotes is data.
    fn fields(row: &str) -> usize {
        let mut n = 1;
        let mut in_quotes = false;
        for c in row.trim_end().chars() {
            match c {
                '"' => in_quotes = !in_quotes,
                ',' if !in_quotes => n += 1,
                _ => {}
            }
        }
        n
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
