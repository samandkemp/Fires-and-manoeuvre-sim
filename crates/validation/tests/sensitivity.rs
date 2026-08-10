//! V71 - the Sobol estimator against a closed form. `docs/EXPERIMENTS.md`.
//!
//! Every number in this project is an abstract placeholder, so the question hanging over
//! every result is whether the conclusion survives the numbers being wrong. Global
//! sensitivity analysis answers it - but only if the estimator is itself trustworthy, and
//! "the indices looked plausible" is not a check.
//!
//! The **Ishigami function** is the standard test case precisely because its Sobol indices
//! are known exactly:
//!
//! ```text
//! f(x) = sin(x1) + a·sin²(x2) + b·x3⁴·sin(x1),     x ~ U(−π, π)
//! ```
//!
//! Its third input is the interesting one: `S1 = 0` exactly - `x3` does **nothing** on its
//! own - while `ST` is large, because it does a great deal *through* `x1`. A one-dial sweep
//! of `x3` would report no effect and be wrong about the model. That gap between `S1` and
//! `ST` is the whole reason this estimator exists, so it is the thing worth gating.
//!
//! This is a gate on the **measuring instrument**, not on the simulation - the same kind as
//! V27's Dijkstra-against-Bellman-Ford. An instrument that cannot recover a known answer
//! cannot be trusted with an unknown one.

use experiments::sensitivity::{
    ishigami, ishigami_analytic, morris_design, morris_indices, sobol_design, sobol_indices,
};

const A: f64 = 7.0;
const B: f64 = 0.1;

/// Ishigami over `U(−π, π)^3`, from unit-cube design coordinates.
fn f(p: &[f64]) -> f64 {
    let pi = std::f64::consts::PI;
    let x: Vec<f64> = p.iter().map(|u| -pi + u * 2.0 * pi).collect();
    ishigami(&x, A, B)
}

// The gate. Sobol indices from Saltelli sampling must land on the analytic values, and in
// particular must report x3's first-order index as ~0 while its total index is large.
#[test]
fn v71_sobol_indices_match_the_ishigami_closed_form() {
    let (k, n) = (3, 1 << 16);
    let design = sobol_design(k, n, 20_260_811);
    let y: Vec<f64> = design.iter().map(|p| f(p)).collect();
    let got = sobol_indices(&y, k, n);
    let want = ishigami_analytic(A, B);

    // Monte-Carlo tolerance: at n = 65,536 the Saltelli estimator is good to a couple of
    // points. Tight enough that a wrong estimator fails, loose enough that a correct one
    // does not depend on the seed.
    const TOL: f64 = 0.03;

    for (i, (ix, (s1, st))) in got.iter().zip(&want).enumerate() {
        assert!(
            (ix.s1 - s1).abs() < TOL,
            "input {i}: S1 = {:.4}, analytic {s1:.4}",
            ix.s1
        );
        assert!(
            (ix.st - st).abs() < TOL,
            "input {i}: ST = {:.4}, analytic {st:.4}",
            ix.st
        );
    }

    // Stated separately because it is the property the whole tool exists for, and a
    // tolerance check above could pass while this was qualitatively wrong.
    assert!(
        got[2].s1.abs() < TOL,
        "x3 has NO first-order effect: a one-dial sweep of it finds nothing (S1 = {:.4})",
        got[2].s1
    );
    assert!(
        got[2].st > 0.2,
        "yet x3 is involved in a fifth of the variance through its interaction with x1 \
         (ST = {:.4}) - that gap is what a sweep is blind to",
        got[2].st
    );
    assert!(
        got[2].st > got[2].s1 + 0.2,
        "so ST must exceed S1 by a wide margin"
    );
}

// A total index is never below a first-order one - an input cannot explain more alone than
// it explains in total. A structural invariant, so it holds whatever the tolerance.
#[test]
fn v71_total_indices_are_never_below_first_order() {
    let (k, n) = (3, 1 << 14);
    let design = sobol_design(k, n, 4242);
    let y: Vec<f64> = design.iter().map(|p| f(p)).collect();
    for (i, ix) in sobol_indices(&y, k, n).iter().enumerate() {
        assert!(
            ix.st >= ix.s1 - 0.02,
            "input {i}: ST {:.4} below S1 {:.4}",
            ix.st,
            ix.s1
        );
    }
}

// Morris is the cheap screen that runs first, and its job is to say what can be ignored. It
// must therefore agree with Sobol about the *ranking* on the same function - the two
// estimators are independent, so agreement is evidence for both.
#[test]
fn v71_morris_screening_agrees_with_sobol_on_what_matters() {
    let (k, r, levels) = (3, 200, 4);
    let design = morris_design(k, r, levels, 99);
    let y: Vec<f64> = design.iter().map(|p| f(p)).collect();
    let m = morris_indices(&design, &y, k, r, levels);

    // x2 enters only as a·sin²(x2) with a = 7, so it is the loudest single dial; x3 alone
    // is quiet. Both are true of the analytic indices too.
    assert!(
        m[1].mu_star > m[2].mu_star,
        "x2 should screen as more influential than x3 ({:.3} vs {:.3})",
        m[1].mu_star,
        m[2].mu_star
    );
    // A dial whose effect depends on the others shows a large sigma next to its mu_star.
    // x1's effect is scaled by x3, so it must not look like a clean linear term.
    assert!(
        m[0].sigma > 0.1 * m[0].mu_star,
        "x1 interacts with x3, so its elementary effects must vary ({:.3} vs {:.3})",
        m[0].sigma,
        m[0].mu_star
    );
}
