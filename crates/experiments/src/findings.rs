//! Re-running the findings the documentation states, and reporting when one has drifted.
//!
//! # Why this exists
//!
//! A measured finding is a claim about a model at a moment. The model then changes, and
//! nothing re-runs the claim - so a number that was right when written goes on being quoted
//! after it stopped being true. This has now happened three times here:
//!
//! * The fire-allocation comparison read "no measurable difference" for two phases after an
//!   unrelated change to the overkill cap made the difference significant. Six documents
//!   carried the stale figure, including the project's own headline example, where the
//!   conclusion had inverted.
//! * The `default` tick cost was quoted at 13.9 µs long after the scenario gained two
//!   drones and began costing ~36. Nothing had regressed; the figure had simply stopped
//!   describing the thing it named.
//! * An earlier version of the allocation finding claimed the opposite result from unpaired
//!   means with no standard errors.
//!
//! None of these is a modelling error. All three are the same bookkeeping failure, and it is
//! the kind that quietly discredits every other number in a project that trades on its
//! numbers.
//!
//! # What a check is
//!
//! A finding names a scenario, a dial, two arms, a metric and an expected **paired**
//! difference with a tolerance. Checking it re-runs both arms over the same seeds and
//! compares. The tolerance is the author's statement of how much the number may move before
//! the prose around it stops being true - not a confidence interval, which the run computes
//! for itself.
//!
//! Findings are deliberately *paired arm against arm*, never each against a shared baseline.
//! Reading a difference across two baselines overstates its error roughly fivefold, and that
//! is exactly how the allocation effect stayed hidden.

use crate::outcome::COLUMNS;
use crate::patch::{self, scenario_with_overrides, Override};
use crate::stats::{self, Paired};
use crate::study::{column, run_study, StudyConfig};
use serde::Deserialize;
use std::path::Path;

/// One documented claim, and the measurement that has to keep supporting it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Short stable handle, used to run one finding on its own.
    pub id: String,
    /// What the documentation asserts, in the words it asserts it.
    pub claim: String,
    /// Every document repeating this number. A drift report lists them, because the work of
    /// fixing a stale finding is mostly finding all the places it was copied to.
    #[serde(default)]
    pub documented_in: Vec<String>,
    /// Scenario the finding is measured on.
    pub scenario: String,
    /// Dotted dial path, scenario or stat-block, exactly as `sweep --param` takes it.
    pub param: String,
    /// Exactly two arms, checked at load. The expected difference is the second minus the
    /// first.
    ///
    /// A `Vec` rather than `[String; 2]` on purpose: the TOML deserializer **silently
    /// truncates** a longer list into a fixed-size array, so a three-arm manifest would
    /// quietly measure the first two and report a confident answer to a question nobody
    /// asked. The length is enforced in [`parse_manifest`] instead, where it can say so.
    pub arms: Vec<String>,
    /// Metric name from [`COLUMNS`].
    pub metric: String,
    /// Paired seeds `0..seeds`.
    pub seeds: u64,
    /// Sim seconds per trial.
    pub until_s: f64,
    /// Expected paired mean difference, `arms[1] - arms[0]`.
    pub expect: f64,
    /// How far it may move before the prose stops being true.
    pub tolerance: f64,
    /// Dials pinned for both arms, `path=value`, as `sweep --set` takes them.
    #[serde(default)]
    pub set: Vec<String>,
}

/// What a manifest file holds.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Every finding in this file.
    #[serde(default, rename = "finding")]
    pub findings: Vec<Finding>,
}

/// How a re-run compared with what the documentation says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Inside tolerance: the prose still describes the model.
    Holds,
    /// Outside tolerance: something changed and the documents have not caught up.
    Drifted,
    /// The finding could not be run at all - a renamed dial, a deleted scenario.
    Broken,
}

/// The result of re-running one finding.
#[derive(Debug, Clone)]
pub struct Checked {
    /// Which finding.
    pub id: String,
    /// Its verdict.
    pub verdict: Verdict,
    /// The paired comparison, absent when the finding could not run.
    pub paired: Option<Paired>,
    /// What was expected.
    pub expect: f64,
    /// The tolerance it was allowed.
    pub tolerance: f64,
    /// Why it could not run, when that is the verdict.
    pub error: Option<String>,
    /// Documents to update if this has drifted.
    pub documented_in: Vec<String>,
}

impl Checked {
    /// How far the measurement sits from the documented value.
    #[must_use]
    pub fn drift(&self) -> Option<f64> {
        self.paired.as_ref().map(|p| p.mean - self.expect)
    }

    /// A one-line report, in the shape the console prints.
    #[must_use]
    pub fn line(&self) -> String {
        match (&self.paired, self.verdict) {
            (Some(p), v) => {
                let tag = match v {
                    Verdict::Holds => "holds",
                    Verdict::Drifted => "DRIFTED",
                    Verdict::Broken => "broken",
                };
                format!(
                    "{:<34} {tag:>8}  measured {:+.3} +- {:.3}, documented {:+.3} (tol {:.3}, drift {:+.3})",
                    self.id,
                    p.mean,
                    p.se,
                    self.expect,
                    self.tolerance,
                    self.drift().unwrap_or(0.0)
                )
            }
            (None, _) => format!(
                "{:<34} {:>8}  {}",
                self.id,
                "BROKEN",
                self.error.as_deref().unwrap_or("could not run")
            ),
        }
    }
}

/// Read a manifest from TOML.
///
/// # Errors
/// A parse failure, including an unknown key - the same `deny_unknown_fields` argument the
/// scenario loader makes, since a misspelt `tolerence` that silently defaulted would make
/// every finding in the file pass.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest = toml::from_str(text).map_err(|e| e.to_string())?;
    for f in &manifest.findings {
        if f.arms.len() != 2 {
            return Err(format!(
                "finding '{}': needs exactly 2 arms to be a paired comparison, got {} ({:?})",
                f.id,
                f.arms.len(),
                f.arms
            ));
        }
        if !f.tolerance.is_finite() || f.tolerance < 0.0 {
            return Err(format!(
                "finding '{}': tolerance must be finite and non-negative, got {}",
                f.id, f.tolerance
            ));
        }
        if f.seeds == 0 {
            return Err(format!("finding '{}': needs at least one seed", f.id));
        }
    }
    Ok(manifest)
}

/// Re-run one finding and compare it with what the documentation claims.
///
/// Never returns an error: a finding that cannot run is a [`Verdict::Broken`] result rather
/// than a failure, because one dead finding must not stop the rest of the manifest being
/// checked. That is the whole value of running them together.
#[must_use]
pub fn check(finding: &Finding, dir: &Path) -> Checked {
    let fail = |msg: String| Checked {
        id: finding.id.clone(),
        verdict: Verdict::Broken,
        paired: None,
        expect: finding.expect,
        tolerance: finding.tolerance,
        error: Some(msg),
        documented_in: finding.documented_in.clone(),
    };

    let Some(metric) = COLUMNS.iter().position(|c| *c == finding.metric) else {
        return fail(format!("unknown metric '{}'", finding.metric));
    };
    let path = dir.join(format!("{}.toml", finding.scenario));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fail(format!("no scenario '{}'", finding.scenario));
    };

    let mut fixed = Vec::with_capacity(finding.set.len());
    for s in &finding.set {
        match Override::parse(s) {
            Ok(o) => fixed.push(o),
            Err(e) => return fail(format!("bad --set '{s}': {e}")),
        }
    }

    let cfg = StudyConfig {
        seeds: finding.seeds,
        until_s: finding.until_s,
        progress: false,
    };

    let mut columns: Vec<Vec<f64>> = Vec::with_capacity(2);
    for arm in &finding.arms {
        let mut overrides = fixed.clone();
        overrides.push(Override {
            path: finding.param.clone(),
            value: patch::parse_value(arm),
        });
        let (lib_overrides, scenario_overrides) = patch::split(&overrides);
        let libs = match patch::libraries_with_overrides(dir, &lib_overrides) {
            Ok(l) => l,
            Err(e) => return fail(format!("{}={arm}: {e}", finding.param)),
        };
        let scn = match scenario_with_overrides(&text, &scenario_overrides) {
            Ok(s) => s,
            Err(e) => return fail(format!("{}={arm}: {e}", finding.param)),
        };
        match run_study(&scn, &libs, cfg) {
            Ok(o) => columns.push(column(&o, metric)),
            Err(e) => return fail(format!("{}={arm}: does not resolve ({e})", finding.param)),
        }
    }

    // `stats::paired(a, b)` is `a - b`, and `expect` is documented as arms[1] minus
    // arms[0] - so the second arm goes first. Getting this backwards inverts every sign in
    // the report while leaving the magnitudes right, which reads exactly like a real drift.
    let paired = stats::paired(&columns[1], &columns[0]);
    let verdict = if (paired.mean - finding.expect).abs() <= finding.tolerance {
        Verdict::Holds
    } else {
        Verdict::Drifted
    };
    Checked {
        id: finding.id.clone(),
        verdict,
        paired: Some(paired),
        expect: finding.expect,
        tolerance: finding.tolerance,
        error: None,
        documented_in: finding.documented_in.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one() -> &'static str {
        r#"
        [[finding]]
        id = "demo"
        claim = "a claim"
        documented_in = ["docs/X.md"]
        scenario = "fire_allocation"
        param = "sim.allocation"
        arms = ["greedy", "optimal"]
        metric = "red_cleared_s"
        seeds = 4
        until_s = 60.0
        expect = 0.4
        tolerance = 0.2
        "#
    }

    #[test]
    fn a_manifest_round_trips() {
        let m = parse_manifest(one()).expect("parses");
        assert_eq!(m.findings.len(), 1);
        assert_eq!(m.findings[0].arms, ["greedy", "optimal"]);
        assert_eq!(m.findings[0].documented_in, ["docs/X.md"]);
    }

    // The `deny_unknown_fields` argument, applied to the checker itself: a misspelt
    // tolerance that silently took a default would make every finding in the file pass,
    // which is worse than having no checker at all.
    #[test]
    fn a_misspelt_key_is_refused() {
        let bad = one().replace("tolerance", "tolerence");
        let err = parse_manifest(&bad).expect_err("must not parse");
        assert!(
            err.contains("tolerence") || err.contains("unknown"),
            "got: {err}"
        );
    }

    // Three arms is not a paired comparison. This is checked explicitly rather than by the
    // type, because `[String; 2]` does NOT enforce it: the TOML deserializer truncates the
    // list and hands back the first two without complaint, so a three-arm manifest would
    // measure two of them and report the answer with confidence.
    #[test]
    fn exactly_two_arms_are_required() {
        let three = one().replace(r#"["greedy", "optimal"]"#, r#"["a", "b", "c"]"#);
        assert_ne!(three, one(), "the fixture text must actually have changed");
        let err = parse_manifest(&three).expect_err("three arms must not parse");
        assert!(err.contains("exactly 2 arms"), "got: {err}");
    }

    #[test]
    fn a_zero_seed_finding_is_refused() {
        let none = one().replace("seeds = 4", "seeds = 0");
        let err = parse_manifest(&none).expect_err("zero seeds must not parse");
        assert!(err.contains("at least one seed"), "got: {err}");
    }

    #[test]
    fn a_verdict_needs_the_measurement_inside_tolerance() {
        let f: Finding = parse_manifest(one()).unwrap().findings.remove(0);
        let inside = Checked {
            id: f.id.clone(),
            verdict: Verdict::Holds,
            paired: Some(stats::from_diffs(&[0.45, 0.45, 0.45, 0.45])),
            expect: f.expect,
            tolerance: f.tolerance,
            error: None,
            documented_in: vec![],
        };
        assert!((inside.drift().unwrap() - 0.05).abs() < 1e-9);
        assert!(inside.line().contains("holds"));
    }

    // The sign convention, pinned. `expect` is arms[1] minus arms[0]; an inverted
    // comparison keeps every magnitude correct and flips every sign, which is
    // indistinguishable from a genuine drift in the report and was exactly the first bug
    // this checker had.
    #[test]
    fn the_difference_is_the_second_arm_minus_the_first() {
        let mut f: Finding = parse_manifest(one()).unwrap().findings.remove(0);
        f.arms = vec!["independent".to_owned(), "greedy".to_owned()];
        f.seeds = 12;
        f.until_s = 200.0;
        f.expect = 0.0;
        f.tolerance = f64::MAX; // verdict is irrelevant here; the sign is the subject
        let out = check(&f, Path::new("../../scenarios"));
        let mean = out.paired.expect("ran").mean;
        assert!(
            mean < 0.0,
            "coordinating (arms[1]) must clear Red SOONER than the old rule (arms[0]), so              the difference is negative; got {mean:+.3}"
        );
    }

    #[test]
    fn a_broken_finding_reports_rather_than_panicking() {
        let mut f: Finding = parse_manifest(one()).unwrap().findings.remove(0);
        f.metric = "not_a_metric".to_owned();
        let out = check(&f, Path::new("../../scenarios"));
        assert_eq!(out.verdict, Verdict::Broken);
        assert!(out.line().contains("BROKEN"));
    }
}
