//! Weapon–target assignment: which shooter engages which target.
//! Spec: `docs/DESIGN.md` §10.2. Gates: V56.
//!
//! Pure functions over a payoff matrix — no `Sim`, no terrain, no randomness. The sim
//! builds the matrix from its own fires model and calls [`solve`]; everything here is a
//! combinatorial optimisation that can be tested on its own.
//!
//! # The problem
//!
//! Rows are shooters, columns are **slots**. A target with `E` elements offers up to `E`
//! slots, so several shooters can be sent against one target without the assignment
//! collapsing to one-shooter-one-target and leaving the rest idle. Each slot is worth
//! less than the last (see `docs/DESIGN.md` §10.2): a second shooter on a target adds
//! less than the first did, because the first may already have destroyed it.
//!
//! Turning diminishing returns into extra columns is what keeps this a plain linear
//! assignment problem instead of needing a bespoke submodular solver.
//!
//! # Two solvers, on purpose
//!
//! [`hungarian`] is optimal. [`greedy`] is the obvious "repeatedly take the best
//! remaining cell" heuristic. Greedy is not dead code: it is the baseline that turns
//! "the optimal solver is worth having" from an assumption into a measured number, which
//! `experiments/allocation_gap` reports.

/// A payoff below this counts as ineligible — the pairing is not allowed at all.
///
/// A finite sentinel, not `-inf`: the Hungarian algorithm subtracts potentials, and
/// `inf - inf` is `NaN`.
///
/// The sentinel never reaches the solver's arithmetic either. It is mapped to **zero**
/// first, because a huge magnitude would destroy the real payoffs it sits beside —
/// `1e18 + 10.0 == 1e18` in `f64`, so every matching with the same number of forbidden
/// cells would score identically and the solver would pick among them arbitrarily.
/// Mapping to zero is exact rather than a fudge: see [`hungarian`].
pub const INELIGIBLE: f64 = -1.0e18;

/// The payoff for a pairing that must never be chosen.
#[must_use]
pub fn ineligible() -> f64 {
    INELIGIBLE
}

/// Is this payoff a real option?
#[must_use]
pub fn is_eligible(payoff: f64) -> bool {
    payoff > INELIGIBLE / 2.0
}

/// Which slot each shooter was given, or `None` if it was left idle.
pub type Assignment = Vec<Option<usize>>;

/// Which solver to use.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Solver {
    /// Kuhn–Munkres: the optimal assignment.
    #[default]
    Optimal,
    /// Repeatedly take the best remaining cell. The baseline the optimum is measured
    /// against.
    Greedy,
    /// Each shooter independently takes its own best slot, ignoring the others — the
    /// pre-Phase-10 behaviour, kept so the cost of *not* coordinating is measurable too.
    Independent,
}

/// Solve the assignment with the chosen solver. `payoff[i][j]` is what shooter `i`
/// contributes in slot `j`; use [`ineligible`] for pairings that are not allowed.
#[must_use]
pub fn solve(payoff: &[Vec<f64>], solver: Solver) -> Assignment {
    match solver {
        Solver::Optimal => hungarian(payoff),
        Solver::Greedy => greedy(payoff),
        Solver::Independent => independent(payoff),
    }
}

/// Total payoff of an assignment. Ineligible or unassigned rows contribute nothing.
#[must_use]
pub fn total(payoff: &[Vec<f64>], assignment: &Assignment) -> f64 {
    assignment
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| slot.map(|j| payoff[i][j]))
        .filter(|p| is_eligible(*p))
        .sum()
}

/// Every shooter takes its own best slot, with no regard for what anyone else does.
///
/// Slots are *not* exclusive here — this reproduces the pre-Phase-10 rule where each unit
/// chose independently, so two shooters can pile onto the same target. Kept as the
/// baseline that shows what coordination buys.
#[must_use]
pub fn independent(payoff: &[Vec<f64>]) -> Assignment {
    payoff
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .filter(|(_, p)| is_eligible(**p))
                // `>` keeps the first of equal maxima, so ties break on the lower index.
                .fold(None, |best: Option<(usize, f64)>, (j, &p)| match best {
                    Some((_, bp)) if bp >= p => best,
                    _ => Some((j, p)),
                })
                .map(|(j, _)| j)
        })
        .collect()
}

/// Repeatedly commit the best remaining (shooter, slot) pair.
///
/// Each shooter and each slot is used at most once. Ties break on the lower shooter index
/// then the lower slot index, so the result is deterministic.
#[must_use]
pub fn greedy(payoff: &[Vec<f64>]) -> Assignment {
    let (n, m) = dimensions(payoff);
    let mut assignment: Assignment = vec![None; n];
    let mut slot_taken = vec![false; m];
    let mut shooter_taken = vec![false; n];

    for _ in 0..n.min(m) {
        let mut best: Option<(usize, usize, f64)> = None;
        for (i, row) in payoff.iter().enumerate().take(n) {
            if shooter_taken[i] {
                continue;
            }
            for (j, &p) in row.iter().enumerate().take(m) {
                if slot_taken[j] || !is_eligible(p) {
                    continue;
                }
                if best.is_none_or(|(_, _, bp)| p > bp) {
                    best = Some((i, j, p));
                }
            }
        }
        let Some((i, j, _)) = best else { break };
        assignment[i] = Some(j);
        shooter_taken[i] = true;
        slot_taken[j] = true;
    }
    assignment
}

/// The optimal assignment, by the Kuhn–Munkres (Hungarian) algorithm.
///
/// `O(n²m)` with the potentials formulation, which handles a rectangular matrix directly
/// — there are usually far more slots than shooters, and padding to a square would waste
/// most of the work.
///
/// The algorithm minimises, so payoffs are negated on the way in. Rows are added one at a
/// time, each extending the alternating tree until it reaches a free column.
///
/// # Forbidden pairings, and idle shooters
///
/// Kuhn–Munkres produces a **perfect** matching — every row gets a column. What we
/// actually want is a maximum-weight matching that may leave a shooter idle, and that
/// forbids some pairings outright. Both fall out of one substitution: forbidden cells are
/// scored **0** for the solver.
///
/// That is exact, not an approximation, given two conditions this module requires:
/// eligible payoffs are non-negative, and rows ≤ columns (guaranteed by the transpose
/// above). Any partial matching then extends to a perfect one using only zero-weight
/// cells without changing its total, so the best perfect matching and the best partial
/// matching have the same value. Afterwards, assignments sitting on a forbidden or
/// worthless cell are simply dropped — a shooter that would contribute nothing is idle.
///
/// # Panics
/// Debug builds assert the non-negativity requirement; a negative eligible payoff would
/// silently break the argument above.
#[must_use]
pub fn hungarian(payoff: &[Vec<f64>]) -> Assignment {
    let (n, m) = dimensions(payoff);
    if n == 0 || m == 0 {
        return vec![None; n];
    }
    debug_assert!(
        payoff
            .iter()
            .flatten()
            .all(|&p| !is_eligible(p) || p >= 0.0),
        "eligible payoffs must be non-negative; see `hungarian`'s forbidden-pairing note"
    );
    // More shooters than slots: solve the transpose so rows ≤ columns, then flip back.
    if n > m {
        let transposed: Vec<Vec<f64>> = (0..m)
            .map(|j| (0..n).map(|i| payoff[i][j]).collect())
            .collect();
        let slot_to_shooter = hungarian(&transposed);
        let mut out = vec![None; n];
        for (j, shooter) in slot_to_shooter.iter().enumerate() {
            if let Some(i) = shooter {
                out[*i] = Some(j);
            }
        }
        return out;
    }

    const INF: f64 = f64::INFINITY;
    // 1-indexed with a sentinel row/column 0, as the standard formulation is written.
    // Forbidden cells score 0 here — see the doc comment for why that is exact.
    let cost = |i: usize, j: usize| {
        let p = payoff[i - 1][j - 1];
        if is_eligible(p) {
            -p
        } else {
            0.0
        }
    };
    let mut u = vec![0.0f64; n + 1]; // row potentials
    let mut v = vec![0.0f64; m + 1]; // column potentials
    let mut row_of = vec![0usize; m + 1]; // which row owns each column
    let mut path = vec![0usize; m + 1]; // alternating-tree parent

    for row in 1..=n {
        row_of[0] = row;
        let mut j0 = 0usize;
        let mut min_slack = vec![INF; m + 1];
        let mut used = vec![false; m + 1];

        loop {
            used[j0] = true;
            let i0 = row_of[j0];
            let mut delta = INF;
            let mut j1 = 0usize;
            for j in 1..=m {
                if used[j] {
                    continue;
                }
                let slack = cost(i0, j) - u[i0] - v[j];
                if slack < min_slack[j] {
                    min_slack[j] = slack;
                    path[j] = j0;
                }
                if min_slack[j] < delta {
                    delta = min_slack[j];
                    j1 = j;
                }
            }
            if !delta.is_finite() {
                break; // no reachable column; this row stays unassigned
            }
            for j in 0..=m {
                if used[j] {
                    u[row_of[j]] += delta;
                    v[j] -= delta;
                } else {
                    min_slack[j] -= delta;
                }
            }
            j0 = j1;
            if row_of[j0] == 0 {
                break;
            }
        }
        // Walk the alternating path back, flipping the matching as we go.
        while j0 != 0 {
            let j1 = path[j0];
            row_of[j0] = row_of[j1];
            j0 = j1;
        }
    }

    let mut assignment: Assignment = vec![None; n];
    for j in 1..=m {
        let row = row_of[j];
        if row == 0 {
            continue;
        }
        // Drop matches that were only taken because the matching must be complete: a
        // forbidden pairing, or one worth nothing. Either way the shooter is idle.
        let p = payoff[row - 1][j - 1];
        if is_eligible(p) && p > 0.0 {
            assignment[row - 1] = Some(j - 1);
        }
    }
    assignment
}

/// `(rows, columns)`, treating a ragged matrix as its narrowest row.
fn dimensions(payoff: &[Vec<f64>]) -> (usize, usize) {
    let n = payoff.len();
    let m = payoff.iter().map(Vec::len).min().unwrap_or(0);
    (n, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive optimum over all injective shooter→slot maps. Only usable for tiny
    /// instances, which is exactly what it is for: the reference V56 checks against.
    fn brute_force(payoff: &[Vec<f64>]) -> f64 {
        let (n, m) = dimensions(payoff);
        let mut used = vec![false; m];
        fn walk(payoff: &[Vec<f64>], i: usize, n: usize, m: usize, used: &mut Vec<bool>) -> f64 {
            if i == n {
                return 0.0;
            }
            // Leaving a shooter idle is always allowed.
            let mut best = walk(payoff, i + 1, n, m, used);
            for j in 0..m {
                if used[j] || !is_eligible(payoff[i][j]) {
                    continue;
                }
                used[j] = true;
                best = best.max(payoff[i][j] + walk(payoff, i + 1, n, m, used));
                used[j] = false;
            }
            best
        }
        walk(payoff, 0, n, m, &mut used)
    }

    /// A small deterministic pseudo-random matrix, so the sweep covers many shapes
    /// without a dependency on `rand` inside a unit test.
    fn matrix(seed: u64, n: usize, m: usize, ineligible_every: u64) -> Vec<Vec<f64>> {
        let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = move || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as f64 / f64::from(u32::MAX >> 1)
        };
        (0..n)
            .map(|_| {
                (0..m)
                    .map(|_| {
                        let x = next();
                        if ineligible_every > 0
                            && ((x * 100.0) as u64).is_multiple_of(ineligible_every)
                        {
                            ineligible()
                        } else {
                            x * 10.0
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn hungarian_matches_brute_force_on_small_instances() {
        for seed in 0..40u64 {
            for n in 1..=5usize {
                for m in 1..=5usize {
                    let p = matrix(seed, n, m, 7);
                    let got = total(&p, &hungarian(&p));
                    let want = brute_force(&p);
                    assert!(
                        (got - want).abs() < 1e-9,
                        "seed {seed} {n}x{m}: hungarian {got} != brute force {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn hungarian_is_never_worse_than_greedy() {
        for seed in 0..60u64 {
            for (n, m) in [(3, 3), (4, 6), (6, 4), (7, 7), (2, 9)] {
                let p = matrix(seed, n, m, 5);
                let h = total(&p, &hungarian(&p));
                let g = total(&p, &greedy(&p));
                assert!(
                    h >= g - 1e-9,
                    "seed {seed} {n}x{m}: greedy {g} beat optimal {h}"
                );
            }
        }
    }

    #[test]
    fn assignments_are_injective_and_eligible() {
        for solver in [Solver::Optimal, Solver::Greedy] {
            for seed in 0..40u64 {
                let p = matrix(seed, 5, 7, 4);
                let a = solve(&p, solver);
                let mut seen = [false; 7];
                for (i, slot) in a.iter().enumerate() {
                    let Some(j) = slot else { continue };
                    assert!(!seen[*j], "slot {j} used twice by {solver:?}");
                    seen[*j] = true;
                    assert!(
                        is_eligible(p[i][*j]),
                        "{solver:?} chose an ineligible pairing"
                    );
                }
            }
        }
    }

    #[test]
    fn a_fully_ineligible_row_is_left_idle() {
        let p = vec![vec![ineligible(), ineligible()], vec![1.0, 2.0]];
        for solver in [Solver::Optimal, Solver::Greedy, Solver::Independent] {
            let a = solve(&p, solver);
            assert_eq!(a[0], None, "{solver:?} assigned an impossible shooter");
            assert_eq!(a[1], Some(1), "{solver:?} missed the best slot");
        }
    }

    #[test]
    fn greedy_can_be_beaten() {
        // The classic trap: greedy grabs the 9 and strands the rest.
        let p = vec![vec![9.0, 8.0], vec![8.0, 0.0]];
        assert!((total(&p, &greedy(&p)) - 9.0).abs() < 1e-9);
        assert!((total(&p, &hungarian(&p)) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn empty_inputs_do_not_panic() {
        assert!(hungarian(&[]).is_empty());
        assert!(greedy(&[]).is_empty());
        let no_slots: Vec<Vec<f64>> = vec![vec![], vec![]];
        assert_eq!(hungarian(&no_slots), vec![None, None]);
    }
}
