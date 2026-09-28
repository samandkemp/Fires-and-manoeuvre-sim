//! The input contract (`docs/THEORY.md` §7.6): values the model cannot run on, refused
//! at load rather than answered wrongly at run time.
//!
//! Separated from the schema because it is a different kind of statement. The schema
//! says what a scenario may contain; this says what it may *mean*, and it is the half a
//! reader goes looking for when a sweep produces something impossible.
//!
//! The same failure in different clothes: `deny_unknown_fields` refuses a key the schema
//! does not know, and the three helpers below refuse a *value* the model cannot run on. A
//! dial outside its domain does not crash - it quietly produces a study of a different
//! question, or, for the two clock dials, does not terminate at all.

use super::{ScenarioError, SimConfig};

/// Reject a dial that must be finite and strictly positive (zero included in the refusal).
pub(super) fn require_positive(name: &str, value: f32) -> Result<(), ScenarioError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(ScenarioError::Invalid(format!(
            "{name} must be positive and finite (got {value})"
        )));
    }
    Ok(())
}

/// Reject a dial that must be finite and non-negative.
pub(super) fn require_non_negative(name: &str, value: f32) -> Result<(), ScenarioError> {
    if !value.is_finite() || value < 0.0 {
        return Err(ScenarioError::Invalid(format!(
            "{name} must be finite and not negative (got {value})"
        )));
    }
    Ok(())
}

/// Reject a dial that must be a probability in `[0, 1]`.
pub(super) fn require_probability(name: &str, value: f32) -> Result<(), ScenarioError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ScenarioError::Invalid(format!(
            "{name} must be a probability in [0, 1] (got {value})"
        )));
    }
    Ok(())
}

impl SimConfig {
    /// Refuse a `[sim]` block the loop cannot run.
    ///
    /// Two of these do not merely give a wrong answer - they **fail to terminate**, which
    /// is why validating them is worth more than the rest put together:
    ///
    /// - `dt_s = 0` leaves the clock where it is, so [`crate::sim::Sim::run_until`] never
    ///   reaches its target;
    /// - `epoch_s = 0` makes `time_s / epoch_s` infinite, and the cast to `u64` **saturates**
    ///   rather than wrapping, so the epoch loop is handed `u64::MAX` boundaries to resolve
    ///   and hangs on the first tick.
    ///
    /// Both are reachable from `experiments/sweep`, which by design can set any dotted path
    /// in the file - `--param sim.epoch_s --from 0` is an ordinary-looking sweep.
    pub(super) fn validate(&self) -> Result<(), ScenarioError> {
        require_positive("[sim] dt_s", self.dt_s)?;
        require_positive("[sim] epoch_s", self.epoch_s)?;
        require_probability("[sim] p_suppress", self.p_suppress)?;
        require_probability("[sim] track_maintain_p", self.track_maintain_p)?;
        require_probability("[sim] suppressed_fire_factor", self.suppressed_fire_factor)?;
        require_non_negative("[sim] track_hold_s", self.track_hold_s)?;
        require_non_negative("[sim] recover_per_s", self.recover_per_s)?;
        require_non_negative("[sim] suppression_radius_m", self.suppression_radius_m)?;
        if self.allocation_horizon == 0 {
            return Err(ScenarioError::Invalid(
                "[sim] allocation_horizon must be at least 1 (1 = score only this epoch)".into(),
            ));
        }
        require_non_negative("[sim] fire_risk_weight", self.fire_risk_weight)?;
        require_non_negative("[sim] risk_weight", self.risk_weight)?;
        require_non_negative("[sim] repath_margin", self.repath_margin)?;
        if self.belief_cells == 0 {
            return Err(ScenarioError::Invalid(
                "[sim] belief_cells must be at least 1".into(),
            ));
        }
        Ok(())
    }
}
