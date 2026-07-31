//! Zero-sum matrix games by fictitious play. Spec: `docs/DESIGN.md` §6.2.
//! Gates: V32–V36, against hand-solvable games.
//!
//! Each round both players best-respond to the opponent's empirical play. For two-player
//! zero-sum games the time-average strategies converge to a Nash equilibrium (Robinson
//! 1951). No LP dependency needed, and watching it converge is itself instructive.

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
