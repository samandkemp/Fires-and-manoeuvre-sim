//! Zero-sum matrix games solved by fictitious play. Specified in `docs/DESIGN.md` §6.2;
//! validated by V32–V36 against hand-solvable games.
//!
//! Fictitious play alternates best responses to the opponent's empirical play. For a
//! two-player zero-sum game the time-average strategies converge to a Nash equilibrium
//! and the value converges (Robinson 1951) — a pure algorithm needing no LP dependency,
//! and the convergence is itself an OR demonstration.

use ndarray::Array2;

/// The solved equilibrium of a zero-sum matrix game.
#[derive(Clone, Debug)]
pub struct GameSolution {
    /// Game value to the row (maximising) player.
    pub value: f32,
    /// Row player's mixed strategy (sums to 1).
    pub row_strategy: Vec<f32>,
    /// Column player's mixed strategy (sums to 1).
    pub col_strategy: Vec<f32>,
    /// `v_high − v_low`: how tightly the value is bracketed (→ 0 with more iterations).
    pub value_gap: f32,
}

/// Solve the zero-sum game with payoff matrix `payoff` (rows = maximiser's strategies,
/// columns = minimiser's), running `iterations` rounds of fictitious play.
///
/// # Panics
/// If `payoff` has no rows or columns.
#[must_use]
pub fn solve_zero_sum(payoff: &Array2<f32>, iterations: usize) -> GameSolution {
    let (m, n) = payoff.dim();
    assert!(m > 0 && n > 0, "payoff matrix must be non-empty");

    // Cumulative payoff of each of our actions against the opponent's play so far.
    let mut row_util = vec![0.0f64; m]; // row's payoff for each row vs col history
    let mut col_util = vec![0.0f64; n]; // col's payoff for each col vs row history
    let mut row_counts = vec![0.0f64; m];
    let mut col_counts = vec![0.0f64; n];

    for _ in 0..iterations.max(1) {
        let i = argmax(&row_util); // row maximises
        let j = argmin(&col_util); // col minimises
        row_counts[i] += 1.0;
        col_counts[j] += 1.0;
        // Fold the just-chosen plays into both cumulative-utility vectors.
        for (jj, cu) in col_util.iter_mut().enumerate() {
            *cu += f64::from(payoff[[i, jj]]);
        }
        for (ii, ru) in row_util.iter_mut().enumerate() {
            *ru += f64::from(payoff[[ii, j]]);
        }
    }

    let t = iterations.max(1) as f64;
    let row_strategy: Vec<f32> = row_counts.iter().map(|&c| (c / t) as f32).collect();
    let col_strategy: Vec<f32> = col_counts.iter().map(|&c| (c / t) as f32).collect();

    // Value bracket: the row player, committing to `row_strategy`, guarantees at least
    // v_low against its worst column; the column player guarantees at most v_high.
    let v_low = (0..n)
        .map(|j| {
            (0..m)
                .map(|i| f64::from(row_strategy[i]) * f64::from(payoff[[i, j]]))
                .sum::<f64>()
        })
        .fold(f64::INFINITY, f64::min);
    let v_high = (0..m)
        .map(|i| {
            (0..n)
                .map(|j| f64::from(payoff[[i, j]]) * f64::from(col_strategy[j]))
                .sum::<f64>()
        })
        .fold(f64::NEG_INFINITY, f64::max);

    GameSolution {
        value: ((v_low + v_high) / 2.0) as f32,
        row_strategy,
        col_strategy,
        value_gap: (v_high - v_low) as f32,
    }
}

/// Index of the largest element (first on ties).
fn argmax(v: &[f64]) -> usize {
    v.iter()
        .enumerate()
        .fold(0, |best, (i, &x)| if x > v[best] { i } else { best })
}

/// Index of the smallest element (first on ties).
fn argmin(v: &[f64]) -> usize {
    v.iter()
        .enumerate()
        .fold(0, |best, (i, &x)| if x < v[best] { i } else { best })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::arr2;

    const ITERS: usize = 100_000;

    // V32: matching pennies — value 0, both play (½, ½).
    #[test]
    fn v32_matching_pennies() {
        let a = arr2(&[[1.0f32, -1.0], [-1.0, 1.0]]);
        let s = solve_zero_sum(&a, ITERS);
        assert!(s.value.abs() < 0.02, "value {} should be ~0", s.value);
        for p in s.row_strategy.iter().chain(&s.col_strategy) {
            assert!((p - 0.5).abs() < 0.03, "strategy weight {p} should be ~0.5");
        }
        assert!(
            s.value_gap < 0.05,
            "bracket should close (gap {})",
            s.value_gap
        );
    }

    // V33: rock–paper–scissors — value 0, uniform.
    #[test]
    fn v33_rock_paper_scissors() {
        let a = arr2(&[[0.0f32, -1.0, 1.0], [1.0, 0.0, -1.0], [-1.0, 1.0, 0.0]]);
        let s = solve_zero_sum(&a, ITERS);
        assert!(s.value.abs() < 0.02, "value {} should be ~0", s.value);
        for p in s.row_strategy.iter().chain(&s.col_strategy) {
            assert!((p - 1.0 / 3.0).abs() < 0.03, "weight {p} should be ~1/3");
        }
    }

    // V34: a pure saddle point — deterministic equilibrium at its value.
    #[test]
    fn v34_saddle_point() {
        // Row maximises: row 0 dominates (4>2, 3>1). Col minimises: col 1 (3<4, 1<2).
        // Saddle at (0, 1) = 3.
        let a = arr2(&[[4.0f32, 3.0], [2.0, 1.0]]);
        let s = solve_zero_sum(&a, 5_000);
        assert!(
            (s.value - 3.0).abs() < 0.05,
            "value {} should be 3",
            s.value
        );
        assert!(s.row_strategy[0] > 0.98, "row should commit to strategy 0");
        assert!(s.col_strategy[1] > 0.98, "col should commit to strategy 1");
    }

    // V35: a strictly dominated strategy gets ~zero weight.
    #[test]
    fn v35_strict_dominance() {
        // Rows 0,1 are matching pennies; row 2 is strictly worse everywhere.
        let a = arr2(&[[1.0f32, -1.0], [-1.0, 1.0], [-2.0, -2.0]]);
        let s = solve_zero_sum(&a, ITERS);
        assert!(
            s.row_strategy[2] < 0.01,
            "dominated strategy weight {} should be ~0",
            s.row_strategy[2]
        );
        assert!(s.value.abs() < 0.03, "value {} should be ~0", s.value);
    }

    // V36: a skew-symmetric (fair) game has value 0 and a closing bracket.
    #[test]
    fn v36_skew_symmetric() {
        // A[i][j] = i − j is skew-symmetric (A = −Aᵀ, zero diagonal).
        let a = Array2::from_shape_fn((4, 4), |(i, j)| i as f32 - j as f32);
        let s = solve_zero_sum(&a, ITERS);
        assert!(
            s.value.abs() < 0.02,
            "fair game value {} should be 0",
            s.value
        );
        assert!(
            s.value_gap < 0.1,
            "bracket should close (gap {})",
            s.value_gap
        );
    }
}
