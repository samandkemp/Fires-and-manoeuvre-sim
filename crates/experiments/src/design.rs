//! Factorial designs: vary several dials at once and ask whether they interact.
//!
//! [`crate::study`] runs one arm. `sweep` runs several arms of **one** dial and reports each
//! against the first. This runs the full Cartesian product of several dials and reports two
//! things a one-dial sweep cannot produce:
//!
//! * a **main effect** - what a factor does, averaged over every level of the others;
//! * an **interaction** - whether the effect of one factor *depends* on another's level.
//!
//! The second is the reason this exists. The `fires_c2` investigation needed a 2×2 over the
//! overkill cap and acquisition speed, it was hand-stitched from four separate sweeps, and
//! the interaction turned out to be the dominant effect: the cap mattered enormously when
//! targets were scarce and hardly at all when they were not. A design that can only vary one
//! dial cannot see that, and reporting two main effects would have described neither.
//!
//! # Everything stays paired
//!
//! Every cell runs the **same seed set**, so a contrast is formed seed by seed and only then
//! averaged. That is what gives a main effect and an interaction an honest standard error
//! rather than the difference of two independent means. It is the same discipline
//! [`crate::stats`] enforces for `sweep`, applied to a harder shape.

use crate::outcome::Outcome;
use crate::stats::{from_diffs, Paired};
use std::fmt::Write as _;

/// One dial and the levels it is to be set to.
#[derive(Clone, Debug)]
pub struct Factor {
    /// Dotted path, exactly as `sweep --param` takes: a scenario field or a stat-block dial.
    pub path: String,
    /// The levels, as written on the command line. Level 0 is the **baseline** every effect
    /// is measured against.
    pub levels: Vec<String>,
}

impl Factor {
    /// Parse `path=v1,v2,v3`. Values split on commas outside brackets, so a list-valued dial
    /// survives (`priority=["c2","all"],["all"]` is two levels, not three).
    ///
    /// # Errors
    /// A message naming what was wrong, for a missing `=` or fewer than two levels - a
    /// factor with one level is not a factor, and silently accepting it would report a main
    /// effect of exactly zero as though it meant something.
    pub fn parse(arg: &str) -> Result<Self, String> {
        let (path, list) = arg
            .split_once('=')
            .ok_or_else(|| format!("--factor needs PATH=v1,v2 (got '{arg}')"))?;
        let levels = crate::patch::split_values(list);
        if levels.len() < 2 {
            return Err(format!(
                "--factor {path} needs at least two levels to be a factor (got {})",
                levels.len()
            ));
        }
        Ok(Self {
            path: path.trim().to_owned(),
            levels,
        })
    }
}

/// Every combination of levels, as an index per factor.
///
/// The **last** factor varies fastest, so the cells read like an odometer and a 2×3 design
/// comes out in the order a reader expects.
#[must_use]
pub fn cells(factors: &[Factor]) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new()];
    for f in factors {
        out = out
            .into_iter()
            .flat_map(|prefix| {
                (0..f.levels.len()).map(move |l| {
                    let mut next = prefix.clone();
                    next.push(l);
                    next
                })
            })
            .collect();
    }
    out
}

/// A finished design: the factors, the cells in [`cells`] order, and one metric column per
/// cell holding that cell's value for every seed.
pub struct Factorial {
    /// The factors, in the order their level indices appear in each cell.
    pub factors: Vec<Factor>,
    /// Level indices per cell, in [`cells`] order.
    pub cells: Vec<Vec<usize>>,
    /// `values[cell][seed]` - the metric for that cell on that seed.
    pub values: Vec<Vec<f64>>,
}

impl Factorial {
    /// Build from the outcomes of each cell, in [`cells`] order.
    ///
    /// # Panics
    /// If a cell ran a different number of seeds from the others, which would make every
    /// contrast below silently unpaired.
    #[must_use]
    pub fn new(
        factors: Vec<Factor>,
        cells: Vec<Vec<usize>>,
        outcomes: &[Vec<Outcome>],
        metric: usize,
    ) -> Self {
        let values: Vec<Vec<f64>> = outcomes
            .iter()
            .map(|o| o.iter().map(|x| x.values()[metric]).collect())
            .collect();
        if let Some(n) = values.first().map(Vec::len) {
            assert!(
                values.iter().all(|v| v.len() == n),
                "every cell of a factorial must run the same seed set, or nothing below is paired"
            );
        }
        Self {
            factors,
            cells,
            values,
        }
    }

    /// Seeds each cell ran.
    #[must_use]
    pub fn seeds(&self) -> usize {
        self.values.first().map_or(0, Vec::len)
    }

    /// Per seed, the mean over every cell matching `filter`.
    ///
    /// Averaging over the *other* factors is what makes an effect "main" rather than a
    /// simple effect measured at one corner of the design.
    fn marginal(&self, filter: impl Fn(&[usize]) -> bool) -> Vec<f64> {
        let matching: Vec<&Vec<f64>> = self
            .cells
            .iter()
            .zip(&self.values)
            .filter(|(c, _)| filter(c))
            .map(|(_, v)| v)
            .collect();
        let n = matching.len().max(1) as f64;
        (0..self.seeds())
            .map(|s| matching.iter().map(|v| v[s]).sum::<f64>() / n)
            .collect()
    }

    /// What factor `f` at `level` does, relative to its baseline level 0, averaged over
    /// every level of every other factor.
    #[must_use]
    pub fn main_effect(&self, f: usize, level: usize) -> Paired {
        let hi = self.marginal(|c| c[f] == level);
        let lo = self.marginal(|c| c[f] == 0);
        let diffs: Vec<f64> = hi.iter().zip(&lo).map(|(a, b)| a - b).collect();
        from_diffs(&diffs)
    }

    /// Whether factor `a`'s effect depends on factor `b`'s level: the classic difference of
    /// differences, formed per seed.
    ///
    /// Measured across each factor's **range** - its last level against its first - with
    /// every other factor averaged out. For a 2×2 that is the whole interaction. For more
    /// levels it is the corner-to-corner contrast, which is a summary rather than the full
    /// picture; the per-cell CSV is there when the full picture is wanted.
    #[must_use]
    pub fn interaction(&self, a: usize, b: usize) -> Paired {
        let (a_hi, b_hi) = (
            self.factors[a].levels.len() - 1,
            self.factors[b].levels.len() - 1,
        );
        let cell = |ai: usize, bi: usize| self.marginal(move |c| c[a] == ai && c[b] == bi);
        let (hh, lh) = (cell(a_hi, b_hi), cell(0, b_hi));
        let (hl, ll) = (cell(a_hi, 0), cell(0, 0));
        let diffs: Vec<f64> = (0..self.seeds())
            .map(|s| (hh[s] - lh[s]) - (hl[s] - ll[s]))
            .collect();
        from_diffs(&diffs)
    }

    /// The report: main effects, then every two-way interaction.
    #[must_use]
    pub fn report(&self, metric_name: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "\n--- {metric_name}: main effects, averaged over the other factors ---"
        );
        for (fi, f) in self.factors.iter().enumerate() {
            let _ = writeln!(s, "  {} (baseline {})", f.path, f.levels[0]);
            for (li, level) in f.levels.iter().enumerate().skip(1) {
                let _ = writeln!(
                    s,
                    "      = {:<24} {}",
                    level,
                    self.main_effect(fi, li).report()
                );
            }
        }

        if self.factors.len() < 2 {
            let _ = writeln!(
                s,
                "\n(one factor, so no interactions - this is a `sweep` with extra steps)"
            );
            return s;
        }

        let _ = writeln!(
            s,
            "\n--- {metric_name}: two-way interactions, across each factor's range ---"
        );
        let mut any_significant = false;
        for a in 0..self.factors.len() {
            for b in (a + 1)..self.factors.len() {
                let p = self.interaction(a, b);
                any_significant |= p.significant();
                let _ = writeln!(
                    s,
                    "  {} x {}\n      {}",
                    self.factors[a].path,
                    self.factors[b].path,
                    p.report()
                );
            }
        }
        let _ = writeln!(
            s,
            "\n{}",
            if any_significant {
                "An interaction is significant, so read the main effects above with care: a \
                 factor's effect is not the same at every level of the other."
            } else {
                "No interaction is significant, so the main effects above are additive and \
                 each factor can be reasoned about on its own."
            }
        );
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factors(shape: &[usize]) -> Vec<Factor> {
        shape
            .iter()
            .enumerate()
            .map(|(i, n)| Factor {
                path: format!("f{i}"),
                levels: (0..*n).map(|l| l.to_string()).collect(),
            })
            .collect()
    }

    /// Build a design whose metric is a known function of the levels, so the effects it
    /// should report are arithmetic rather than opinion.
    fn synthetic(shape: &[usize], seeds: usize, f: impl Fn(&[usize], usize) -> f64) -> Factorial {
        let facs = factors(shape);
        let cs = cells(&facs);
        let values = cs
            .iter()
            .map(|c| (0..seeds).map(|s| f(c, s)).collect())
            .collect();
        Factorial {
            factors: facs,
            cells: cs,
            values,
        }
    }

    #[test]
    fn cells_are_the_cartesian_product_with_the_last_factor_fastest() {
        let cs = cells(&factors(&[2, 3]));
        assert_eq!(cs.len(), 6);
        assert_eq!(
            cs,
            vec![
                vec![0, 0],
                vec![0, 1],
                vec![0, 2],
                vec![1, 0],
                vec![1, 1],
                vec![1, 2]
            ]
        );
    }

    // An additive metric has main effects equal to its coefficients and no interaction -
    // the case where reasoning about factors separately is valid.
    #[test]
    fn an_additive_metric_has_its_coefficients_and_no_interaction() {
        let d = synthetic(&[2, 2], 16, |c, _| 3.0 * c[0] as f64 + 5.0 * c[1] as f64);
        assert!((d.main_effect(0, 1).mean - 3.0).abs() < 1e-9);
        assert!((d.main_effect(1, 1).mean - 5.0).abs() < 1e-9);
        assert!(
            d.interaction(0, 1).mean.abs() < 1e-9,
            "a sum of separate terms cannot have an interaction"
        );
    }

    // A product term is pure interaction: neither factor does anything on its own at the
    // other's baseline, and the effect appears only when both are raised.
    #[test]
    fn a_product_metric_is_all_interaction() {
        let d = synthetic(&[2, 2], 16, |c, _| 7.0 * (c[0] * c[1]) as f64);
        assert!(
            (d.interaction(0, 1).mean - 7.0).abs() < 1e-9,
            "the whole effect is the interaction"
        );
        // The main effect is the marginal average, so it is half the product term - real,
        // but it describes neither level of the other factor.
        assert!((d.main_effect(0, 1).mean - 3.5).abs() < 1e-9);
    }

    // The point of pairing: a per-seed offset shared by every cell must cancel completely,
    // however large it is. Unpaired, this noise would swamp the effect.
    #[test]
    fn a_shared_per_seed_offset_cancels() {
        let noise = |s: usize| (s as f64) * 1000.0;
        let d = synthetic(&[2, 2], 32, |c, s| 3.0 * c[0] as f64 + noise(s));
        let e = d.main_effect(0, 1);
        assert!((e.mean - 3.0).abs() < 1e-9);
        assert!(
            e.se < 1e-9,
            "shared noise must cancel, leaving no error bar"
        );
    }

    // ---- against the real engine ------------------------------------------------------
    //
    // The tests above check the arithmetic on synthetic numbers. These two check that the
    // arithmetic is being fed the right numbers, which is the other half.

    use crate::study::{run_study, StudyConfig};
    use sim_core::scenario::Libraries;
    use std::path::Path;

    fn fixture() -> Option<(String, Libraries)> {
        let dir = Path::new("../../scenarios");
        let libs = Libraries::load_dir(dir).ok()?;
        let text = std::fs::read_to_string(dir.join("flat_range.toml")).ok()?;
        Some((text, libs))
    }

    /// Run one cell the way `bin/factorial.rs` does: patch, load, study.
    fn cell_outcomes(text: &str, libs: &Libraries, path: &str, value: &str) -> Vec<Outcome> {
        let ov = [crate::patch::Override {
            path: path.to_owned(),
            value: crate::patch::parse_value(value),
        }];
        let scn = crate::patch::scenario_with_overrides(text, &ov).expect("patches");
        run_study(
            &scn,
            libs,
            StudyConfig {
                seeds: 12,
                until_s: 120.0,
                progress: false,
            },
        )
        .expect("resolves")
    }

    /// A one-factor factorial is a `sweep`, and must agree with one **exactly**. Both patch
    /// the same dial and run the same seeds, so any difference would mean one of them is
    /// doing something to the scenario the other is not.
    #[test]
    fn a_one_factor_design_reproduces_a_sweep_exactly() {
        let Some((text, libs)) = fixture() else {
            return; // scenarios/ not present
        };
        let (path, levels) = ("sim.p_suppress", ["0.05", "0.4"]);

        let swept: Vec<Vec<Outcome>> = levels
            .iter()
            .map(|v| cell_outcomes(&text, &libs, path, v))
            .collect();

        let factors = vec![Factor {
            path: path.to_owned(),
            levels: levels.iter().map(|s| (*s).to_owned()).collect(),
        }];
        let cs = cells(&factors);
        assert_eq!(cs, vec![vec![0], vec![1]], "one factor, one cell per level");

        let metric = Outcome::column("red_losses").expect("known metric");
        let d = Factorial::new(factors, cs, &swept, metric);

        // The design's own column must be the sweep's column, value for value.
        for (i, arm) in swept.iter().enumerate() {
            let from_sweep: Vec<f64> = arm.iter().map(|o| o.values()[metric]).collect();
            assert_eq!(d.values[i], from_sweep, "cell {i} disagrees with the sweep");
        }
        // And the main effect must be the paired difference a sweep would report.
        let a: Vec<f64> = swept[1].iter().map(|o| o.values()[metric]).collect();
        let b: Vec<f64> = swept[0].iter().map(|o| o.values()[metric]).collect();
        let expected = crate::stats::paired(&a, &b);
        let got = d.main_effect(0, 1);
        assert!((got.mean - expected.mean).abs() < 1e-12);
        assert!((got.se - expected.se).abs() < 1e-12);
        assert_eq!(got.n, expected.n);
        assert_eq!(got.ties, expected.ties);
    }

    /// A factor the model cannot be affected by must produce no interaction with one that
    /// matters - and, being an exact identity rather than a small effect, every seed ties.
    ///
    /// `suppression_radius_m` is swept against a dial that genuinely does nothing here: the
    /// air-defence overkill cap, on a scenario with no aircraft at all. If the harness were
    /// mispairing cells, this is where it would show as noise.
    #[test]
    fn a_factor_that_cannot_matter_shows_no_interaction() {
        let Some((text, libs)) = fixture() else {
            return;
        };
        let factors = vec![
            Factor {
                path: "sim.p_suppress".to_owned(),
                levels: vec!["0.05".to_owned(), "0.4".to_owned()],
            },
            Factor {
                path: "sim.max_batteries_per_air_target".to_owned(),
                levels: vec!["1".to_owned(), "3".to_owned()],
            },
        ];
        let cs = cells(&factors);
        let mut outcomes = Vec::new();
        for c in &cs {
            let ov: Vec<crate::patch::Override> = c
                .iter()
                .enumerate()
                .map(|(fi, &li)| crate::patch::Override {
                    path: factors[fi].path.clone(),
                    value: crate::patch::parse_value(&factors[fi].levels[li]),
                })
                .collect();
            let scn = crate::patch::scenario_with_overrides(&text, &ov).expect("patches");
            outcomes.push(
                run_study(
                    &scn,
                    &libs,
                    StudyConfig {
                        seeds: 12,
                        until_s: 120.0,
                        progress: false,
                    },
                )
                .expect("resolves"),
            );
        }
        let metric = Outcome::column("red_losses").expect("known metric");
        let d = Factorial::new(factors, cs, &outcomes, metric);

        let inert = d.main_effect(1, 1);
        assert_eq!(
            inert.ties, inert.n,
            "an air-defence dial on a scenario with no aircraft must be an exact identity"
        );
        assert!(inert.mean.abs() < 1e-12);

        let interaction = d.interaction(0, 1);
        assert_eq!(
            interaction.ties, interaction.n,
            "and it therefore cannot interact with anything"
        );
        assert!(interaction.mean.abs() < 1e-12);
    }

    #[test]
    fn a_factor_needs_two_levels() {
        assert!(Factor::parse("sim.dt_s=1.0").is_err());
        assert!(Factor::parse("sim.dt_s").is_err());
        let f = Factor::parse("sim.allocation=greedy,optimal").expect("two levels");
        assert_eq!(f.path, "sim.allocation");
        assert_eq!(f.levels, vec!["greedy", "optimal"]);
    }

    // List-valued dials survive, which is what `patch::split_values` is for.
    #[test]
    fn a_list_valued_level_is_not_split_on_its_own_commas() {
        let f = Factor::parse(r#"blue.doctrine.priority=["c2","all"],["all"]"#).expect("parses");
        assert_eq!(f.levels.len(), 2, "two levels, not three");
        assert_eq!(f.levels[0], r#"["c2","all"]"#);
    }
}
