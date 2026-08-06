//! Means, standard errors, and paired differences.
//!
//! A mean on its own cannot answer the only question a study asks — *is this difference
//! real?* — so nothing in this crate reports one without an error bar beside it.
//!
//! # Why every comparison here is paired
//!
//! Two arms of a study (two solvers, two values of a dial) are run over the **same seed
//! set**, so each seed gives a matched pair `(a_k, b_k)` on the same map with the same
//! dice. The difference is then `d_k = a_k - b_k`, and its standard error is
//!
//! ```text
//!   SE(d̄) = s_d / sqrt(n),   s_d² = Σ(d_k - d̄)² / (n - 1)
//! ```
//!
//! This is the classic variance-reduction technique of **common random numbers**. It works
//! because `Var(a - b) = Var(a) + Var(b) - 2 Cov(a, b)`: the two arms share the map and
//! most of the luck, so `Cov(a, b)` is large and positive and most of the variance cancels.
//! Comparing the two *unpaired* means throws that away and can be an order of magnitude
//! noisier.
//!
//! That is not a hypothetical. Comparing the fire-allocation solvers unpaired once
//! produced a confident finding that greedy beat the optimal assignment; paired over 500
//! seeds the difference was 0.12 s against an SE of 0.5, and the two were *identical* on
//! 438 of the 500 seeds (`docs/DESIGN.md` §10.2). The variance being cancelled was the
//! whole effect.

/// Mean and standard error of a sample.
///
/// `SE = s / sqrt(n)` with `s` the sample standard deviation (Bessel-corrected). Zero for
/// a sample of one, which is honest: one run says nothing about spread.
#[must_use]
pub fn mean_and_se(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = xs.iter().sum::<f64>() / n;
    if n < 2.0 {
        return (mean, 0.0);
    }
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

/// A paired difference and what it is worth believing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Paired {
    /// Mean of `a_k - b_k` over the matched seeds.
    pub mean: f64,
    /// Standard error of that mean.
    pub se: f64,
    /// `mean / se` — the paired t statistic. `|t| > 2` is the usual "worth believing"
    /// line for a sample this size (two-sided, ~5%).
    pub t: f64,
    /// Matched pairs compared.
    pub n: usize,
    /// Pairs where the two arms gave *exactly* the same number. A high count next to a
    /// small mean says the two arms are mostly the same decision, not that the effect is
    /// merely hard to see — which is a different conclusion.
    pub ties: usize,
}

impl Paired {
    /// Is the difference outside the noise? `|t| > 2`, the usual two-sided ~5% line.
    #[must_use]
    pub fn significant(&self) -> bool {
        self.t.abs() > 2.0
    }

    /// A one-line verdict: the difference, its error bar, and whether to believe it.
    #[must_use]
    pub fn report(&self) -> String {
        format!(
            "{:+.3} +- {:.3} (t = {:.1}, n = {}, {} tied) {}",
            self.mean,
            self.se,
            self.t,
            self.n,
            self.ties,
            if self.significant() {
                "significant"
            } else {
                "NOT significant"
            }
        )
    }
}

/// Paired difference `a - b`, seed by seed.
///
/// # Panics
/// If the two samples are different lengths — that means they were not run over the same
/// seed set, and pairing them would be silently wrong rather than merely imprecise.
#[must_use]
pub fn paired(a: &[f64], b: &[f64]) -> Paired {
    assert_eq!(
        a.len(),
        b.len(),
        "paired comparison needs the same seeds in both arms"
    );
    let diffs: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    let ties = diffs.iter().filter(|d| **d == 0.0).count();
    let (mean, se) = mean_and_se(&diffs);
    Paired {
        mean,
        se,
        t: if se > 0.0 { mean / se } else { 0.0 },
        n: diffs.len(),
        ties,
    }
}

/// A metric summarised over a study arm: what it averaged, and how sure that is.
#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    pub mean: f64,
    pub se: f64,
    pub n: usize,
}

impl Summary {
    #[must_use]
    pub fn of(xs: &[f64]) -> Self {
        let (mean, se) = mean_and_se(xs);
        Self {
            mean,
            se,
            n: xs.len(),
        }
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2} +-{:.2}", self.mean, self.se)
    }
}

/// Collapse `-0.0` to `0.0` for output.
///
/// Rust's `f64` sum folds from `-0.0`, not `0.0`, because `-0.0 + x == x` for every `x`
/// whereas `0.0 + (-0.0)` would drop the sign. So a metric summed over an empty log comes
/// out `-0.0` and prints as `-0`, which in a results file reads like a bug.
#[must_use]
pub fn tidy(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_and_se_match_the_closed_form() {
        // s² = 2.5 for [1,2,3,4,5], so SE = sqrt(2.5/5) = 0.7071.
        let (mean, se) = mean_and_se(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!((mean - 3.0).abs() < 1e-12);
        assert!((se - (2.5f64 / 5.0).sqrt()).abs() < 1e-12);
        assert_eq!(mean_and_se(&[]), (0.0, 0.0));
        assert_eq!(mean_and_se(&[7.0]), (7.0, 0.0));
    }

    /// The point of pairing: a constant offset buried in huge shared noise is invisible
    /// unpaired and obvious paired. These two samples differ by exactly 1 everywhere, on
    /// top of a spread of ~30 that both share.
    #[test]
    fn pairing_recovers_an_offset_that_unpaired_means_would_miss() {
        let shared: Vec<f64> = (0..40).map(|k| f64::from(k) * 2.5).collect();
        let a: Vec<f64> = shared.iter().map(|x| x + 1.0).collect();
        let p = paired(&a, &shared);
        assert!((p.mean - 1.0).abs() < 1e-12);
        assert!(p.se < 1e-12, "an exact offset has no spread");
        assert_eq!(p.ties, 0);

        // Unpaired, the same offset is swamped: the SEs alone overlap heavily.
        let (_, se_a) = mean_and_se(&a);
        assert!(se_a > 1.0, "shared spread dwarfs the effect: SE = {se_a}");
    }

    #[test]
    fn ties_are_counted_and_a_null_difference_is_not_significant() {
        let a = [1.0, 2.0, 3.0, 4.0];
        let p = paired(&a, &a);
        assert_eq!(p.ties, 4);
        assert_eq!(p.mean, 0.0);
        assert!(!p.significant());
    }

    #[test]
    #[should_panic(expected = "same seeds")]
    fn mismatched_arms_are_a_bug_not_a_smaller_sample() {
        let _ = paired(&[1.0, 2.0], &[1.0]);
    }
}
