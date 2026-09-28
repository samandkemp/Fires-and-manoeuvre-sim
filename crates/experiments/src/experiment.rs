//! What an experiment *is*, as a value that can be queued, saved, and re-run from a shell.
//!
//! An experiment run by clicking is not reproducible, and a number nobody can reproduce
//! stops being evidence. So the runner does not "do experiments": it builds a [`RunSpec`],
//! shows the command that spec *is*, and then carries it out - so anything set up by hand
//! can be copied to a terminal or pasted into `findings.toml`.
//!
//! That is why the type lives here rather than in the app: it is the definition of the
//! work, testable without a window. A [`Batch`] is an ordered list of specs.

use crate::overrides::split_values;
use std::fmt::Write as _;

/// Which study tool a spec runs, and the arguments only that tool takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// One dial, several values, paired against the first.
    Sweep {
        /// Dotted dial path.
        param: String,
        /// The arms, as they would be typed after `--values`.
        values: Vec<String>,
    },
    /// Several dials at once, reporting main effects and two-way interactions.
    Factorial {
        /// One entry per factor, each `path=v1,v2,...`.
        factors: Vec<String>,
    },
    /// A folder of scenarios, one row each.
    Batch {
        /// Restrict to scenarios whose name contains this, if set.
        only: Option<String>,
    },
}

impl Kind {
    /// The binary that carries this out.
    #[must_use]
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Sweep { .. } => "sweep",
            Self::Factorial { .. } => "factorial",
            Self::Batch { .. } => "batch",
        }
    }
}

/// One experiment, in full.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSpec {
    /// A short name, so a queue of six is readable.
    pub label: String,
    /// Scenario name, without the `.toml`.
    pub scenario: String,
    /// What kind of study, and its own arguments.
    pub kind: Kind,
    /// Metric reported to the console. The CSV always carries all of them.
    pub metric: String,
    /// Paired seeds `0..seeds`.
    pub seeds: u64,
    /// Sim seconds per trial.
    pub until_s: f64,
    /// Dials pinned for every arm, each `path=value`.
    pub fixed: Vec<String>,
}

impl Default for RunSpec {
    fn default() -> Self {
        Self {
            label: "untitled".to_owned(),
            scenario: "default".to_owned(),
            kind: Kind::Sweep {
                param: String::new(),
                values: Vec::new(),
            },
            metric: "red_cleared_s".to_owned(),
            seeds: 200,
            until_s: 300.0,
            fixed: Vec::new(),
        }
    }
}

impl RunSpec {
    /// The exact shell command this spec is.
    ///
    /// Not a summary and not an approximation - what is printed here must run and produce
    /// what the runner produced, or the reproducibility claim is hollow. Arguments carrying a
    /// comma or a bracket are quoted, because a list dial's value legitimately contains both
    /// and an unquoted one would be re-split by the shell.
    #[must_use]
    pub fn command(&self) -> String {
        let mut s = format!(
            "cargo run -p experiments --release --bin {} -- {}",
            self.kind.binary(),
            self.scenario
        );
        match &self.kind {
            Kind::Sweep { param, values } => {
                let _ = write!(s, " --param {param} --values {}", quote(&values.join(",")));
            }
            Kind::Factorial { factors } => {
                for f in factors {
                    let _ = write!(s, " --factor {}", quote(f));
                }
            }
            Kind::Batch { only } => {
                if let Some(o) = only {
                    let _ = write!(s, " --only {}", quote(o));
                }
            }
        }
        // `batch` takes a directory rather than a scenario, and has no metric of its own.
        if !matches!(self.kind, Kind::Batch { .. }) {
            let _ = write!(s, " --metric {}", self.metric);
        }
        let _ = write!(s, " --seeds {} --until {}", self.seeds, self.until_s);
        for f in &self.fixed {
            let _ = write!(s, " --set {}", quote(f));
        }
        s
    }

    /// How many trials this will run, for a queue that has to say how long it will be.
    ///
    /// A sweep is one arm per value; a factorial is the product of its factors' levels. Both
    /// multiply by seeds. `Batch` cannot be counted without reading the directory, so it
    /// reports `None` rather than guessing.
    #[must_use]
    pub fn trials(&self) -> Option<u64> {
        let arms = match &self.kind {
            Kind::Sweep { values, .. } => values.len() as u64,
            Kind::Factorial { factors } => factors
                .iter()
                .map(|f| {
                    f.split_once('=')
                        .map_or(1, |(_, vs)| split_values(vs).len() as u64)
                })
                .product(),
            Kind::Batch { .. } => return None,
        };
        Some(arms * self.seeds)
    }

    /// What is wrong with this spec, if anything.
    ///
    /// Checked before it can be queued, because the failure mode worth avoiding is a batch
    /// left running overnight where the third of six was malformed. The messages name what to
    /// fix rather than what is wrong.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.scenario.trim().is_empty() {
            out.push("choose a scenario".to_owned());
        }
        if self.seeds == 0 {
            out.push("seeds must be at least 1".to_owned());
        }
        if !self.until_s.is_finite() || self.until_s <= 0.0 {
            out.push("`until` must be a positive number of seconds".to_owned());
        }
        match &self.kind {
            Kind::Sweep { param, values } => {
                if param.trim().is_empty() {
                    out.push("pick a dial to sweep".to_owned());
                }
                if values.len() < 2 {
                    out.push("a sweep needs at least two values to compare".to_owned());
                }
            }
            Kind::Factorial { factors } => {
                if factors.len() < 2 {
                    out.push("a factorial needs at least two factors".to_owned());
                }
                for f in factors {
                    if !f.contains('=') {
                        out.push(format!("factor `{f}` needs `path=v1,v2`"));
                    }
                }
            }
            Kind::Batch { .. } => {}
        }
        out
    }

    /// The spec as TOML, for saving a batch next to the results it produced.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut s = format!(
            "[[run]]\nlabel = {:?}\nscenario = {:?}\nmetric = {:?}\nseeds = {}\nuntil_s = {}\n",
            self.label, self.scenario, self.metric, self.seeds, self.until_s
        );
        match &self.kind {
            Kind::Sweep { param, values } => {
                let _ = write!(s, "kind = \"sweep\"\nparam = {param:?}\nvalues = [");
                let _ = writeln!(
                    s,
                    "{}]",
                    values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Kind::Factorial { factors } => {
                let _ = write!(
                    s,
                    "kind = \"factorial\"\nfactors = [{}]\n",
                    factors
                        .iter()
                        .map(|f| format!("{f:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Kind::Batch { only } => {
                let _ = writeln!(s, "kind = \"batch\"");
                if let Some(o) = only {
                    let _ = writeln!(s, "only = {o:?}");
                }
            }
        }
        if !self.fixed.is_empty() {
            let _ = writeln!(
                s,
                "fixed = [{}]",
                self.fixed
                    .iter()
                    .map(|f| format!("{f:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        s
    }
}

/// An ordered list of experiments, run one after another.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    /// The runs, in the order they will be carried out.
    pub runs: Vec<RunSpec>,
}

impl Batch {
    /// Total trials across the batch, and how many runs could not be counted.
    #[must_use]
    pub fn size(&self) -> (u64, usize) {
        let mut total = 0;
        let mut uncounted = 0;
        for r in &self.runs {
            match r.trials() {
                Some(n) => total += n,
                None => uncounted += 1,
            }
        }
        (total, uncounted)
    }

    /// The whole batch as a shell script, in order.
    ///
    /// This is the artefact that makes a queue reproducible: whatever was set up by hand,
    /// this is the file that repeats it.
    #[must_use]
    pub fn to_script(&self) -> String {
        let mut s = String::from(
            "#!/bin/sh\n# Generated by the experiment runner. Every run it queued, in order.\nset -e\n\n",
        );
        for r in &self.runs {
            let _ = writeln!(s, "# {}", r.label);
            let _ = writeln!(s, "{}\n", r.command());
        }
        s
    }

    /// The whole batch as TOML, for reloading into the runner.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# An experiment batch. Reload it in the runner, or read it as a record of what\n\
             # was run - `to_script` turns the same thing into shell commands.\n\n",
        );
        for r in &self.runs {
            s.push_str(&r.to_toml());
            s.push('\n');
        }
        s
    }
}

/// Quote an argument that would otherwise be re-split by a shell.
fn quote(arg: &str) -> String {
    if arg.contains([',', ' ', '[', ']', '"']) {
        format!("'{arg}'")
    } else {
        arg.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sweep() -> RunSpec {
        RunSpec {
            label: "allocation".to_owned(),
            scenario: "fire_allocation".to_owned(),
            kind: Kind::Sweep {
                param: "sim.allocation".to_owned(),
                values: vec!["greedy".to_owned(), "optimal".to_owned()],
            },
            metric: "red_cleared_s".to_owned(),
            seeds: 500,
            until_s: 300.0,
            fixed: Vec::new(),
        }
    }

    /// The reproducibility contract: what the runner shows must be what a shell would run.
    #[test]
    fn a_sweep_spec_is_its_command() {
        let cmd = sweep().command();
        assert!(cmd.contains("--bin sweep -- fire_allocation"), "{cmd}");
        assert!(cmd.contains("--param sim.allocation"), "{cmd}");
        assert!(cmd.contains("--values 'greedy,optimal'"), "{cmd}");
        assert!(cmd.contains("--seeds 500"), "{cmd}");
        assert!(cmd.contains("--metric red_cleared_s"), "{cmd}");
    }

    /// A value list contains commas by definition, and a list-valued dial contains brackets
    /// too. Unquoted, a shell would split them and the command would silently run something
    /// else - which is worse than failing, because it would produce a plausible number.
    #[test]
    fn arguments_that_would_be_resplit_are_quoted() {
        let mut s = sweep();
        s.fixed = vec![r#"blue.doctrine.priority=["c2","armour"]"#.to_owned()];
        let cmd = s.command();
        assert!(
            cmd.contains(r#"--set 'blue.doctrine.priority=["c2","armour"]'"#),
            "{cmd}"
        );
    }

    /// `batch` takes no metric, so emitting one would produce a command that does not run.
    #[test]
    fn a_batch_command_omits_what_batch_does_not_take() {
        let s = RunSpec {
            kind: Kind::Batch { only: None },
            ..sweep()
        };
        let cmd = s.command();
        assert!(!cmd.contains("--metric"), "batch has no --metric: {cmd}");
        assert!(cmd.contains("--bin batch"), "{cmd}");
    }

    #[test]
    fn trial_counts_multiply_out() {
        assert_eq!(sweep().trials(), Some(1000)); // two arms, 500 seeds
        let f = RunSpec {
            kind: Kind::Factorial {
                factors: vec![
                    "sim.allocation=greedy,optimal".to_owned(),
                    "sim.fires_need_c2=false,true".to_owned(),
                ],
            },
            seeds: 100,
            ..sweep()
        };
        assert_eq!(f.trials(), Some(400)); // 2 x 2 cells, 100 seeds
                                           // A batch's size depends on the directory, so it is unknown rather than guessed.
        let b = RunSpec {
            kind: Kind::Batch { only: None },
            ..sweep()
        };
        assert_eq!(b.trials(), None);
    }

    /// The failure worth preventing: a batch left running overnight whose third run was
    /// malformed. Every problem is caught before anything is queued.
    #[test]
    fn a_malformed_spec_says_what_to_fix() {
        let bad = RunSpec {
            scenario: String::new(),
            seeds: 0,
            until_s: 0.0,
            kind: Kind::Sweep {
                param: String::new(),
                values: vec!["only-one".to_owned()],
            },
            ..sweep()
        };
        let problems = bad.problems();
        assert_eq!(problems.len(), 5, "got {problems:?}");
        assert!(problems.iter().any(|p| p.contains("scenario")));
        assert!(problems.iter().any(|p| p.contains("two values")));
    }

    #[test]
    fn a_good_spec_has_no_problems() {
        assert!(sweep().problems().is_empty());
    }

    #[test]
    fn a_batch_reports_its_size_and_what_it_could_not_count() {
        let b = Batch {
            runs: vec![
                sweep(),
                RunSpec {
                    kind: Kind::Batch { only: None },
                    ..sweep()
                },
            ],
        };
        assert_eq!(b.size(), (1000, 1));
    }

    /// The queue's record of itself has to be runnable, not decorative.
    #[test]
    fn a_batch_script_contains_every_command_in_order() {
        let b = Batch {
            runs: vec![
                sweep(),
                RunSpec {
                    label: "second".to_owned(),
                    seeds: 50,
                    ..sweep()
                },
            ],
        };
        let script = b.to_script();
        let first = script.find("--seeds 500").expect("first run");
        let second = script.find("--seeds 50 ").expect("second run");
        assert!(first < second, "order must be preserved");
        assert!(script.contains("# allocation") && script.contains("# second"));
    }

    #[test]
    fn a_spec_round_trips_through_toml_readably() {
        let t = sweep().to_toml();
        assert!(t.contains("kind = \"sweep\""), "{t}");
        assert!(t.contains("param = \"sim.allocation\""), "{t}");
        assert!(t.contains("seeds = 500"), "{t}");
    }
}
