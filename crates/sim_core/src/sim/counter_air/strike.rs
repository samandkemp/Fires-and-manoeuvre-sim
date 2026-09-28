//! Releasing a munition (`docs/THEORY.md` §9.3, §12.3).
//!
//! A drone within `release_range_m` of its aim point drops one, which then resolves as an
//! ordinary §2.3 indirect round through [`super::damage`]. What the aim point *is*, and
//! whether the target is radiating for an anti-radiation seeker to ride, are decided here.

use crate::airframes::TargetSpec;
use crate::sim::{Sim, StrikeEvent};
use crate::{los, weapon_effects};
use glam::Vec2;

impl Sim {
    /// Strike release (`docs/THEORY.md` §9.3): a drone within `release_range_m` of its
    /// aim point drops one munition, which resolves exactly as a §2.3 indirect round.
    pub(in crate::sim) fn resolve_strikes(&mut self) {
        if self.air.is_empty() {
            return;
        }
        let now = self.time_s;
        for a_idx in 0..self.air.len() {
            if !self.air[a_idx].can_strike() {
                continue;
            }
            let Some(aim) = self.strike_aim_point(a_idx) else {
                continue;
            };
            let air = &self.air[a_idx];
            let height = air.actor_height(&self.terrain);
            let range = los::slant_range(&self.terrain, air.pos, height, aim, 0.0);
            if range > air.stats.release_range_m {
                continue;
            }

            let weapon = air.payload.clone().expect("can_strike checked the payload");
            let side = air.side;
            // An anti-radiation munition rides the target's own signal down, so what it
            // hits depends on whether that signal is there (§12.3). For every other weapon
            // `cep_against` returns `cep_m` whatever this says.
            let emitting = self.target_is_emitting(a_idx);
            let sigma = weapon_effects::sigma_from_cep(weapon.cep_against(emitting));
            let burst = weapon_effects::sample_burst(aim, sigma, &mut self.rng);
            let casualties = self.apply_area_damage(burst, &weapon, side);

            let air = &mut self.air[a_idx];
            air.munitions_left -= 1;
            if air.stats.expendable {
                air.alive = false; // a one-way attack munition dies with its target
            }
            self.strike_events.push(StrikeEvent {
                time_s: now,
                air: a_idx,
                aim,
                burst,
                casualties,
            });
        }
    }

    /// A strike drone's aim point: its assigned target if it still exists, otherwise the
    /// final waypoint of its flight plan (`docs/THEORY.md` §9.3).
    ///
    /// A named target that is already dead yields `None` **unless** the airframe is
    /// `autonomous`, in which case it looks for something else worth hitting within release
    /// range. Assignment still wins: a drone with a live assigned target goes for that one,
    /// so orders are not quietly overridden by opportunity.
    pub(super) fn strike_aim_point(&self, air_idx: usize) -> Option<Vec2> {
        // A dead **named** target must yield no release without autonomy - the §7.4
        // identity half of V60. Only the unassigned arm falls through to the flight plan's
        // destination; doing so for a named target would break that identity.
        let autonomous = self.air[air_idx].stats.autonomous;
        match &self.air[air_idx].target {
            Some(TargetSpec::Point(p)) => Some(*p),
            Some(TargetSpec::Named(id)) => self
                .named_ground_asset(id)
                .or_else(|| autonomous.then(|| self.opportune_target(air_idx)).flatten()),
            None => autonomous
                .then(|| self.opportune_target(air_idx))
                .flatten()
                .or_else(|| self.air[air_idx].plan.destination()),
        }
    }

    /// The best thing this drone could hit from where it is, or `None` if nothing qualifies.
    ///
    /// Three constraints:
    ///
    /// * the target must be **located** by the drone's own side - autonomy means acting on
    ///   the side's picture without waiting to be told, not seeing through terrain;
    /// * it must already be within `release_range_m`, since the drone does not divert. This
    ///   is opportunism along the route it was given;
    /// * the choice is ranked by the **same** value function and doctrine the ground fires
    ///   use, so a side ordered to kill command posts first does it with drones too (§13.3).
    ///
    /// Deterministic: ties break on the fixed target-list order, and no randomness is drawn.
    fn opportune_target(&self, air_idx: usize) -> Option<Vec2> {
        let air = &self.air[air_idx];
        let height = air.actor_height(&self.terrain);
        let doctrine = self.doctrine_of(air.side);
        let value_scale = self.threat_scale();

        self.engageable_targets(air.side)
            .into_iter()
            .filter_map(|t| {
                let state = self.target_state(t);
                if !state.located {
                    return None;
                }
                let range = los::slant_range(&self.terrain, air.pos, height, state.pos, 0.0);
                if range > air.stats.release_range_m {
                    return None;
                }
                // Rank exactly as a gun would: doctrine tier first, then value. Using the
                // tier directly rather than a weight keeps strict ordering strict - the
                // same reason §13.2 solves one tier at a time instead of adding a bonus.
                let tier = doctrine.tier_of(&self.target_names(t));
                let value = self.target_value(t, value_scale);
                Some((tier, value, state.pos))
            })
            // Lowest tier first, then highest value. `total_cmp` rather than `partial_cmp`
            // so equal values order deterministically instead of leaving it to NaN rules.
            .min_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)))
            .map(|(_, _, pos)| pos)
    }

    /// Is this strike drone's assigned target currently radiating (`docs/THEORY.md`
    /// §12.3)?
    ///
    /// True only for a **named air-defence battery** that is alive, has an organic radar
    /// and is using it. A unit, a post or a bare map point emits nothing an ARM could ride,
    /// so an ARM aimed at one is blind by definition.
    ///
    /// `emitting` is the counter and it costs the radar: a battery under EMCON detects
    /// nothing through it, cannot cue itself and contributes no coverage. Survive the
    /// missile, or see the raid coming - not both.
    ///
    /// Not `self_cue`, which says only whose track the battery acts on. One flag for both
    /// would let it take the protection of going dark while still seeing (§12.5, V69).
    fn target_is_emitting(&self, air_idx: usize) -> bool {
        let Some(TargetSpec::Named(id)) = &self.air[air_idx].target else {
            return false;
        };
        self.air_defence
            .iter()
            .find(|d| d.id == *id)
            .is_some_and(|d| {
                d.alive() && d.emitting && d.sensor_idx.is_some_and(|s| self.sensor_active(s))
            })
    }

    /// Where the ground asset called `id` is, if it is still alive.
    ///
    /// Searches units, then air-defence batteries, then C2 posts. Ids are unique within a
    /// scenario, so one namespace is enough - and it means naming a SAM or a command post
    /// as a strike target simply works, which is what makes SEAD expressible in a
    /// scenario file rather than needing new syntax (`docs/THEORY.md` §12).
    fn named_ground_asset(&self, id: &str) -> Option<Vec2> {
        if let Some(u) = self.units.iter().find(|u| u.id == id && u.alive()) {
            return Some(u.pos);
        }
        if let Some(ad) = self.air_defence.iter().find(|a| a.id == id && a.alive()) {
            return Some(ad.pos);
        }
        self.c2
            .iter()
            .find(|c| c.id == id && c.alive())
            .map(|c| c.pos)
    }
}
