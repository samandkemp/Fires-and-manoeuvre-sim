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
