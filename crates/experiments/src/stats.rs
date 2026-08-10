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

/// Summarise a difference that has **already been formed** per seed.
///
/// [`paired`] subtracts two columns and calls this. A factorial contrast cannot: a main
/// effect is a difference of *marginal means* over several cells, and an interaction is a
/// difference of differences, both computed seed by seed before there is anything to
/// compare. They arrive here instead, so the t statistic, the tie count and the
/// significance line mean exactly what they mean everywhere else.
#[must_use]
pub fn from_diffs(diffs: &[f64]) -> Paired {
    let ties = diffs.iter().filter(|d| **d == 0.0).count();
    let (mean, se) = mean_and_se(diffs);
    Paired {
        mean,
        se,
        t: if se > 0.0 { mean / se } else { 0.0 },
        n: diffs.len(),
        ties,
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
    from_diffs(&diffs)
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

/// A quantile with a bootstrap confidence interval.
///
/// Means come with a standard error everywhere in this crate. Tails did not, and the tail
/// is often the question: *how bad is a bad day* is a different question from *what happens
/// on average*, and for a saturating raid it is the more useful one. A median leakage of
/// zero with a 95th percentile of four is a defence that usually holds and occasionally
/// does not — which a mean of 0.4 describes to nobody.
#[derive(Clone, Copy, Debug, Default)]
pub struct Quantile {
    /// Which quantile, in `[0, 1]`.
    pub p: f64,
    /// The sample quantile itself.
    pub value: f64,
    /// Lower end of the interval.
    pub lo: f64,
    /// Upper end of the interval.
    pub hi: f64,
    /// Sample size it was estimated from.
    pub n: usize,
}

impl std::fmt::Display for Quantile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "p{:.0} = {:.3} [{:.3}, {:.3}]",
            self.p * 100.0,
            self.value,
            self.lo,
            self.hi
        )
    }
}

/// The `p`-quantile of a sample, by linear interpolation between order statistics.
///
/// # Panics
/// If `xs` is empty — there is no quantile of nothing, and returning zero would be a
/// number that looks like an answer.
#[must_use]
pub fn quantile(xs: &[f64], p: f64) -> f64 {
    assert!(!xs.is_empty(), "no quantile of an empty sample");
    let mut s: Vec<f64> = xs.to_vec();
    s.sort_by(f64::total_cmp);
    if s.len() == 1 {
        return s[0];
    }
    let h = p.clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    let frac = h - lo as f64;
    s[lo] + frac * (s[hi] - s[lo])
}

/// A quantile with a percentile-bootstrap interval at `1 - alpha`.
///
/// Resampling rather than a formula because a simulation outcome is rarely anything a
/// closed-form quantile interval would apply to: leakage is bounded below at zero and often
/// discrete, clear-time is censored at the run length. The bootstrap asks the sample what
/// its own sampling distribution looks like, and makes no shape assumption at all.
///
/// Deterministic given `seed`, like everything else here — a confidence interval that moved
/// between runs of the same data would be worse than none.
///
/// # Panics
/// If `xs` is empty, or `resamples` is zero.
#[must_use]
pub fn quantile_ci(xs: &[f64], p: f64, resamples: usize, alpha: f64, seed: u64) -> Quantile {
    assert!(!xs.is_empty(), "no quantile of an empty sample");
    assert!(resamples > 0, "a bootstrap needs resamples");
    use rand::{Rng, SeedableRng};
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);

    let mut boot = Vec::with_capacity(resamples);
    let mut draw = vec![0.0; xs.len()];
    for _ in 0..resamples {
        for slot in &mut draw {
            *slot = xs[rng.random_range(0..xs.len())];
        }
        boot.push(quantile(&draw, p));
    }
    Quantile {
        p,
        value: quantile(xs, p),
        lo: quantile(&boot, alpha / 2.0),
        hi: quantile(&boot, 1.0 - alpha / 2.0),
        n: xs.len(),
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

    // ---- quantiles and their bootstrap intervals ---------------------------------------

    /// `U(0, 1)` has quantile `p` at exactly `p`, which is the simplest closed form there
    /// is to check an estimator against.
    fn uniform_sample(n: usize, seed: u64) -> Vec<f64> {
        use rand::{Rng, SeedableRng};
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        (0..n).map(|_| rng.random::<f64>()).collect()
    }

    #[test]
    fn a_quantile_of_a_known_sample_is_the_order_statistic() {
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((quantile(&xs, 0.0) - 1.0).abs() < 1e-12);
        assert!((quantile(&xs, 0.5) - 3.0).abs() < 1e-12);
        assert!((quantile(&xs, 1.0) - 5.0).abs() < 1e-12);
        // Interpolated between order statistics, not rounded to one.
        assert!((quantile(&xs, 0.125) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn quantiles_of_a_uniform_sample_land_near_p() {
        let xs = uniform_sample(4000, 7);
        for p in [0.1, 0.5, 0.9, 0.95] {
            let q = quantile(&xs, p);
            assert!((q - p).abs() < 0.03, "p{p}: got {q}, U(0,1) says {p}");
        }
    }

    /// The property a confidence interval is *for*: it must cover the truth about
    /// `1 - alpha` of the time. Checked by repetition rather than asserted once, because a
    /// single interval containing the answer says nothing about the method.
    #[test]
    fn the_bootstrap_interval_covers_the_truth_about_as_often_as_it_claims() {
        let (p, alpha, reps) = (0.9, 0.10, 200);
        let covered = (0..reps)
            .filter(|&r| {
                let xs = uniform_sample(300, 1000 + r as u64);
                let q = quantile_ci(&xs, p, 400, alpha, 55 + r as u64);
                q.lo <= p && p <= q.hi
            })
            .count();
        let rate = covered as f64 / reps as f64;
        // Nominal 90%. The percentile bootstrap is approximate for a quantile, so the band
        // is generous — but an interval that covered 50% or 100% of the time would be
        // useless in opposite ways, and both are excluded.
        assert!(
            (0.80..=0.99).contains(&rate),
            "nominal 90% interval covered {:.0}% of the time",
            rate * 100.0
        );
    }

    #[test]
    fn a_bootstrap_interval_brackets_its_own_estimate_and_is_reproducible() {
        let xs = uniform_sample(500, 3);
        let a = quantile_ci(&xs, 0.95, 500, 0.05, 9);
        assert!(a.lo <= a.value && a.value <= a.hi, "{a}");
        let b = quantile_ci(&xs, 0.95, 500, 0.05, 9);
        assert!(
            (a.lo - b.lo).abs() < 1e-12 && (a.hi - b.hi).abs() < 1e-12,
            "same data and seed must give the same interval"
        );
    }

    /// More data must not make the interval wider. The check that the estimator is
    /// converging on something rather than wandering.
    #[test]
    fn more_data_narrows_the_interval() {
        let wide = quantile_ci(&uniform_sample(100, 21), 0.5, 400, 0.05, 1);
        let tight = quantile_ci(&uniform_sample(4000, 21), 0.5, 400, 0.05, 1);
        assert!(
            tight.hi - tight.lo < wide.hi - wide.lo,
            "40x the data should narrow the interval ({:.4} vs {:.4})",
            tight.hi - tight.lo,
            wide.hi - wide.lo
        );
    }

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
