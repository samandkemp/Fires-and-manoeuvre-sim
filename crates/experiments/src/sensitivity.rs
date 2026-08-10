//! Global sensitivity analysis: which dials actually drive the answer?
//!
//! Every number in this project is an abstract placeholder. That is a deliberate choice —
//! the models are the product and the numbers are knobs — but it leaves one question
//! hanging over every result: **does it matter that these numbers are invented?**
//!
//! A `sweep` cannot answer it. It varies one dial with the rest held at whatever the
//! scenario happened to say, so it measures a slice through a space it never explores. If
//! two dials interact, or if the scenario's own values sit somewhere unrepresentative, the
//! slice can be badly unlike the whole.
//!
//! Two estimators here, cheap-then-thorough:
//!
//! * **Morris elementary effects** — a screening design. One-factor-at-a-time steps along
//!   random trajectories through the dial space. `mu_star` ranks influence, `sigma` flags a
//!   dial whose effect is non-linear or depends on the others. Cost is `r * (k + 1)` runs
//!   for `k` dials, so it is affordable first and its job is to say what to ignore.
//! * **Sobol indices** — a variance decomposition, via Saltelli sampling. `S1` is the
//!   fraction of output variance a dial explains alone; `ST` includes everything it is
//!   involved in. `ST - S1` is therefore how much of a dial's influence runs *through* its
//!   interactions, which is exactly what a one-dial sweep is blind to.
//!
//! # Why this is its own tool
//!
//! A Sobol study is a different object from a paired comparison. Different sampling
//! (Saltelli, not a shared seed set), different output (a variance decomposition, not a
//! difference with an error bar), different question. Folding it into `sweep` would force
//! one report format to serve two incompatible purposes.
//!
//! # Sampling, and why it is not the seeded RNG
//!
//! The dial-space sample is drawn from its own seeded `ChaCha8Rng`, separate from the
//! simulation's. A study must be reproducible in *both* — the same study seed gives the
//! same design, and each design point still runs the same simulation seeds. Mixing them
//! would make a design point's dial values depend on how many trials ran before it.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// One dial and the range it is explored over.
#[derive(Clone, Debug)]
pub struct Dial {
    /// Dotted path, exactly as `sweep --param` takes.
    pub path: String,
    /// Inclusive lower bound.
    pub lo: f64,
    /// Inclusive upper bound.
    pub hi: f64,
}

impl Dial {
    /// Map a unit-cube coordinate to this dial's range.
    #[must_use]
    pub fn at(&self, u: f64) -> f64 {
        self.lo + u.clamp(0.0, 1.0) * (self.hi - self.lo)
    }
}

/// What one dial contributed, by whichever estimator produced it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Index {
    /// Morris: mean absolute elementary effect — how much this dial moves the answer.
    /// Sobol: unused.
    pub mu_star: f64,
    /// Morris: spread of the elementary effects. Large next to `mu_star` means the dial's
    /// effect depends on where the others are — non-linearity or interaction.
    pub sigma: f64,
    /// Sobol first-order index: the share of output variance this dial explains **alone**.
    pub s1: f64,
    /// Sobol total index: the share it is involved in, including every interaction.
    /// `st - s1` is the part a one-dial sweep cannot see.
    pub st: f64,
}

/// A design point: one set of unit-cube coordinates, one per dial.
pub type Point = Vec<f64>;

// ---------------------------------------------------------------------------------------
// Morris screening
// ---------------------------------------------------------------------------------------

/// `r` trajectories of `k + 1` points each: start at random, then step one dial at a time.
///
/// Returns the points in evaluation order. [`morris_indices`] expects them back in the same
/// order, which is what lets the caller evaluate them however it likes — in parallel, or on
/// a cluster — without this module knowing anything about simulations.
#[must_use]
pub fn morris_design(k: usize, trajectories: usize, levels: usize, seed: u64) -> Vec<Point> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let delta = if levels < 2 {
        0.5
    } else {
        levels as f64 / (2.0 * (levels as f64 - 1.0))
    };
    let mut out = Vec::with_capacity(trajectories * (k + 1));
    for _ in 0..trajectories {
        // Start on the grid, low enough that a +delta step stays inside the cube.
        let mut base: Point = (0..k)
            .map(|_| {
                let g = rng.random_range(0..levels.max(2)) as f64 / (levels.max(2) - 1) as f64;
                g.min(1.0 - delta)
            })
            .collect();
        // A random order, so no dial is systematically stepped first.
        let mut order: Vec<usize> = (0..k).collect();
        for i in (1..k).rev() {
            order.swap(i, rng.random_range(0..=i));
        }
        out.push(base.clone());
        for &d in &order {
            base[d] += delta;
            out.push(base.clone());
        }
    }
    out
}

/// Elementary effects from the outputs of [`morris_design`], in the same order.
///
/// # Panics
/// If `outputs` is not `trajectories * (k + 1)` long — a mismatch means the points and the
/// answers have drifted apart, and every index below would be attributed to the wrong dial.
#[must_use]
pub fn morris_indices(
    design: &[Point],
    outputs: &[f64],
    k: usize,
    trajectories: usize,
    levels: usize,
) -> Vec<Index> {
    assert_eq!(
        design.len(),
        outputs.len(),
        "one output per design point, or the effects are misattributed"
    );
    assert_eq!(
        design.len(),
        trajectories * (k + 1),
        "unexpected design size"
    );
    let delta = if levels < 2 {
        0.5
    } else {
        levels as f64 / (2.0 * (levels as f64 - 1.0))
    };

    let mut effects: Vec<Vec<f64>> = vec![Vec::new(); k];
    for t in 0..trajectories {
        let base = t * (k + 1);
        for step in 0..k {
            let (before, after) = (base + step, base + step + 1);
            // Which dial moved is read from the design itself rather than remembered, so a
            // caller that reorders or filters points cannot silently mislabel an effect.
            let moved = (0..k).find(|&d| (design[after][d] - design[before][d]).abs() > 1e-12);
            if let Some(d) = moved {
                effects[d].push((outputs[after] - outputs[before]) / delta);
            }
        }
    }

    effects
        .into_iter()
        .map(|es| {
            if es.is_empty() {
                return Index::default();
            }
            let n = es.len() as f64;
            let mu_star = es.iter().map(|e| e.abs()).sum::<f64>() / n;
            let mean = es.iter().sum::<f64>() / n;
            let var = es.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / n;
            Index {
                mu_star,
                sigma: var.sqrt(),
                ..Index::default()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Sobol indices, by Saltelli sampling
// ---------------------------------------------------------------------------------------

/// Saltelli's design: `n * (k + 2)` points as `A`, `B`, then `A_B^i` for each dial `i`.
///
/// `A_B^i` is `A` with column `i` taken from `B`. Comparing `f(A)` against `f(A_B^i)`
/// isolates what dial `i` alone explains; comparing `f(B)` against it isolates everything
/// *except* dial `i`, which is where the total index comes from.
#[must_use]
pub fn sobol_design(k: usize, n: usize, seed: u64) -> Vec<Point> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let draw = |rng: &mut ChaCha8Rng| -> Vec<Point> {
        (0..n)
            .map(|_| (0..k).map(|_| rng.random::<f64>()).collect())
            .collect()
    };
    let a = draw(&mut rng);
    let b = draw(&mut rng);

    let mut out = Vec::with_capacity(n * (k + 2));
    out.extend(a.iter().cloned());
    out.extend(b.iter().cloned());
    for i in 0..k {
        for row in 0..n {
            let mut p = a[row].clone();
            p[i] = b[row][i];
            out.push(p);
        }
    }
    out
}

/// First-order and total Sobol indices from the outputs of [`sobol_design`].
///
/// Uses the Saltelli/Jansen estimators, which are the ones that behave when an index is
/// near zero:
///
/// * `S1 = (1/n) Σ f(B)·(f(A_B) − f(A)) / Var`
/// * `ST = (1/2n) Σ (f(A) − f(A_B))² / Var`
///
/// # Panics
/// If `outputs` is not `n * (k + 2)` long.
#[must_use]
pub fn sobol_indices(outputs: &[f64], k: usize, n: usize) -> Vec<Index> {
    assert_eq!(
        outputs.len(),
        n * (k + 2),
        "unexpected Saltelli design size"
    );
    let fa = &outputs[0..n];
    let fb = &outputs[n..2 * n];

    // Variance over both base samples: more data, and it is the same population.
    let all: Vec<f64> = fa.iter().chain(fb).copied().collect();
    let mean = all.iter().sum::<f64>() / all.len() as f64;
    let var = all.iter().map(|y| (y - mean).powi(2)).sum::<f64>() / all.len() as f64;
    if var <= 0.0 {
        // A constant output has no variance to apportion; every index is zero rather than
        // NaN, which would otherwise poison the whole report.
        return vec![Index::default(); k];
    }

    (0..k)
        .map(|i| {
            let fab = &outputs[(2 + i) * n..(3 + i) * n];
            let s1 = fb
                .iter()
                .zip(fab)
                .zip(fa)
                .map(|((b, ab), a)| b * (ab - a))
                .sum::<f64>()
                / (n as f64 * var);
            let st = fa
                .iter()
                .zip(fab)
                .map(|(a, ab)| (a - ab).powi(2))
                .sum::<f64>()
                / (2.0 * n as f64 * var);
            Index {
                s1,
                st,
                ..Index::default()
            }
        })
        .collect()
}

/// The Ishigami function — the standard sensitivity-analysis test case, because its Sobol
/// indices are known in closed form.
///
/// `f(x) = sin(x1) + a·sin²(x2) + b·x3⁴·sin(x1)`, with each `x ~ U(−π, π)`. Note that `x3`
/// has **zero** first-order index and a large total one: it does nothing on its own and a
/// great deal through `x1`. A one-dial sweep of `x3` would find nothing, which is precisely
/// the blind spot this module exists to cover.
#[must_use]
pub fn ishigami(x: &[f64], a: f64, b: f64) -> f64 {
    x[0].sin() + a * x[1].sin().powi(2) + b * x[2].powi(4) * x[0].sin()
}

/// Analytic Sobol indices of [`ishigami`] — `(S1, ST)` per input.
#[must_use]
pub fn ishigami_analytic(a: f64, b: f64) -> Vec<(f64, f64)> {
    let pi = std::f64::consts::PI;
    let v1 = 0.5 * (1.0 + b * pi.powi(4) / 5.0).powi(2);
    let v2 = a * a / 8.0;
    let v13 = b * b * pi.powi(8) * (1.0 / 18.0 - 1.0 / 50.0);
    let v = v1 + v2 + v13;
    vec![(v1 / v, (v1 + v13) / v), (v2 / v, v2 / v), (0.0, v13 / v)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morris_design_has_the_right_shape_and_moves_one_dial_at_a_time() {
        let (k, r, levels) = (4, 6, 4);
        let d = morris_design(k, r, levels, 7);
        assert_eq!(d.len(), r * (k + 1));
        for t in 0..r {
            let base = t * (k + 1);
            for step in 0..k {
                let moved = (0..k)
                    .filter(|&i| (d[base + step + 1][i] - d[base + step][i]).abs() > 1e-12)
                    .count();
                assert_eq!(moved, 1, "a Morris step moves exactly one dial");
            }
            // Every dial moves exactly once per trajectory.
            let mut seen = vec![0usize; k];
            for step in 0..k {
                for i in 0..k {
                    if (d[base + step + 1][i] - d[base + step][i]).abs() > 1e-12 {
                        seen[i] += 1;
                    }
                }
            }
            assert!(seen.iter().all(|&c| c == 1), "each dial steps once");
        }
        assert!(
            d.iter().flatten().all(|u| (0.0..=1.0).contains(u)),
            "trajectories must stay inside the unit cube"
        );
    }

    #[test]
    fn morris_ranks_an_inert_dial_last() {
        let (k, r, levels) = (3, 40, 4);
        let d = morris_design(k, r, levels, 11);
        // Dial 1 does nothing at all.
        let y: Vec<f64> = d
            .iter()
            .map(|p| 10.0 * p[0] + 0.0 * p[1] + 2.0 * p[2])
            .collect();
        let idx = morris_indices(&d, &y, k, r, levels);
        assert!(
            idx[1].mu_star < 1e-9,
            "an inert dial has no elementary effect"
        );
        assert!(
            idx[0].mu_star > idx[2].mu_star,
            "and the ranking is by influence"
        );
    }

    #[test]
    fn sobol_design_has_the_right_shape() {
        let (k, n) = (3, 8);
        let d = sobol_design(k, n, 5);
        assert_eq!(d.len(), n * (k + 2));
        // A_B^i differs from A in column i only.
        for i in 0..k {
            for row in 0..n {
                let a = &d[row];
                let ab = &d[(2 + i) * n + row];
                for c in 0..k {
                    if c == i {
                        continue;
                    }
                    assert!((a[c] - ab[c]).abs() < 1e-12, "column {c} should match A");
                }
            }
        }
    }

    /// An additive model has first-order indices summing to 1 and no interaction, so
    /// `ST == S1` for every input. If the estimator disagreed here it would be reporting
    /// interactions that do not exist.
    #[test]
    fn sobol_on_a_purely_additive_model() {
        let (k, n) = (3, 4096);
        let d = sobol_design(k, n, 3);
        let y: Vec<f64> = d
            .iter()
            .map(|p| 3.0 * p[0] + 1.0 * p[1] + 0.0 * p[2])
            .collect();
        let idx = sobol_indices(&y, k, n);
        let total: f64 = idx.iter().map(|i| i.s1).sum();
        assert!(
            (total - 1.0).abs() < 0.05,
            "additive S1 must sum to ~1, got {total}"
        );
        for (i, ix) in idx.iter().enumerate() {
            assert!(
                (ix.st - ix.s1).abs() < 0.05,
                "input {i}: no interaction, so ST should equal S1 ({} vs {})",
                ix.st,
                ix.s1
            );
        }
        assert!(idx[2].st < 0.05, "the inert input explains nothing");
    }
}
