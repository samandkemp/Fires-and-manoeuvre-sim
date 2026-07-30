//! V32-V39 - the zero-sum game solver (docs/DESIGN.md §6).
//!
//! Fixtures come from the `validation` crate; the gates reach sim_core through its
//! public API only.

use ndarray::{arr2, Array2};
use sim_core::game::*;

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
