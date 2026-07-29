//! Suppression: a per-unit discrete Markov chain `Free → Suppressed → Pinned` driven by
//! near-miss volume and decaying over time. Specified in `docs/DESIGN.md` §4.3;
//! validated by V28–V29 (chain) and V31 (fire gating, in `sim.rs`).

/// A unit's suppression state. Near-misses step it up; time steps it down.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Suppression {
    /// Unsuppressed: full fire and movement.
    #[default]
    Free,
    /// Heads-down: outgoing fire is degraded; may still move.
    Suppressed,
    /// Pinned: cannot fire, cannot move.
    Pinned,
}

impl Suppression {
    /// One level worse (saturating at `Pinned`).
    #[must_use]
    pub fn step_up(self) -> Self {
        match self {
            Suppression::Free => Suppression::Suppressed,
            Suppression::Suppressed | Suppression::Pinned => Suppression::Pinned,
        }
    }

    /// One level better (saturating at `Free`).
    #[must_use]
    pub fn step_down(self) -> Self {
        match self {
            Suppression::Pinned => Suppression::Suppressed,
            Suppression::Suppressed | Suppression::Free => Suppression::Free,
        }
    }

    /// Can this unit move? (False only when Pinned.)
    #[must_use]
    pub fn can_move(self) -> bool {
        self != Suppression::Pinned
    }

    /// Outgoing-fire effectiveness multiplier: `1` Free, `suppressed_factor` Suppressed,
    /// `0` Pinned.
    #[must_use]
    pub fn fire_effectiveness(self, suppressed_factor: f32) -> f32 {
        match self {
            Suppression::Free => 1.0,
            Suppression::Suppressed => suppressed_factor,
            Suppression::Pinned => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};

    fn rng(seed: u64) -> crate::SimRng {
        crate::SimRng::seed_from_u64(seed)
    }

    // V28: under a constant up-rate β and down-rate μ, long-run occupancy matches the
    // birth–death stationary distribution π_k ∝ (β/μ)^k, k ∈ {0,1,2}.
    #[test]
    fn v28_stationary_distribution() {
        let (beta, mu, dt) = (0.04f32, 0.08f32, 0.5f32);
        let p_up = 1.0 - (-beta * dt).exp();
        let p_down = 1.0 - (-mu * dt).exp();
        let steps = 4_000_000u32;

        let mut r = rng(1);
        let mut state = Suppression::Free;
        let mut occ = [0u64; 3];
        for _ in 0..steps {
            occ[state as usize] += 1;
            // Up then down; with small dt double-events are negligible.
            if state != Suppression::Pinned && r.random::<f32>() < p_up {
                state = state.step_up();
            }
            if state != Suppression::Free && r.random::<f32>() < p_down {
                state = state.step_down();
            }
        }

        let ratio = beta / mu; // 0.5
        let z = 1.0 + ratio + ratio * ratio;
        let analytic = [1.0 / z, ratio / z, ratio * ratio / z];
        for k in 0..3 {
            let emp = occ[k] as f32 / steps as f32;
            assert!(
                (emp - analytic[k]).abs() < 0.01,
                "state {k}: empirical {emp:.3} vs stationary {:.3}",
                analytic[k]
            );
        }
    }

    // V29: with no incoming fire, mean recovery time Pinned → Free is two exponential
    // steps at rate μ, i.e. 2/μ.
    #[test]
    fn v29_recovery_time() {
        let (mu, dt) = (0.1f32, 0.25f32);
        let p_down = 1.0 - (-mu * dt).exp();
        let trials = 40_000u32;

        let mut r = rng(2);
        let mut total_time = 0.0f64;
        for _ in 0..trials {
            let mut state = Suppression::Pinned;
            let mut t = 0.0f32;
            while state != Suppression::Free {
                t += dt;
                if r.random::<f32>() < p_down {
                    state = state.step_down();
                }
            }
            total_time += f64::from(t);
        }
        let mean = total_time / f64::from(trials);
        let expected = 2.0 / f64::from(mu); // 20 s
                                            // Discrete stepping biases the mean up by ~dt per step; allow a small band.
        assert!(
            (mean - expected).abs() < 1.0,
            "mean recovery {mean:.2} s vs 2/μ = {expected:.2} s"
        );
    }

    #[test]
    fn effects_and_saturation() {
        assert_eq!(Suppression::Free.step_down(), Suppression::Free);
        assert_eq!(Suppression::Pinned.step_up(), Suppression::Pinned);
        assert_eq!(
            Suppression::Free.step_up().step_up().step_up(),
            Suppression::Pinned
        );
        assert!(!Suppression::Pinned.can_move());
        assert!(Suppression::Suppressed.can_move());
        assert_eq!(Suppression::Pinned.fire_effectiveness(0.4), 0.0);
        assert_eq!(Suppression::Suppressed.fire_effectiveness(0.4), 0.4);
        assert_eq!(Suppression::Free.fire_effectiveness(0.4), 1.0);
    }
}
