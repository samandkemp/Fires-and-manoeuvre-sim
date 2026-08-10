//! Releasing a munition (`docs/DESIGN.md` §9.3, §12.3).
//!
//! A drone within `release_range_m` of its aim point drops one, which then resolves as an
//! ordinary §2.3 indirect round through [`super::damage`]. What the aim point *is*, and
//! whether the target is radiating for an anti-radiation seeker to ride, are decided here.

use crate::air::TargetSpec;
use crate::sim::{Sim, StrikeEvent};
use crate::{fires, los};
use glam::Vec2;

impl Sim {
    /// Strike release (`docs/DESIGN.md` §9.3): a drone within `release_range_m` of its
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
            let sigma = fires::sigma_from_cep(weapon.cep_against(emitting));
            let burst = fires::sample_burst(aim, sigma, &mut self.rng);
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
    /// final waypoint of its flight plan (`docs/DESIGN.md` §9.3). A named target that is
    /// already dead yields `None` — the drone does not re-target itself, by design.
    pub(super) fn strike_aim_point(&self, air_idx: usize) -> Option<Vec2> {
        match &self.air[air_idx].target {
            Some(TargetSpec::Point(p)) => Some(*p),
            Some(TargetSpec::Named(id)) => self.named_ground_asset(id),
            None => self.air[air_idx].plan.destination(),
        }
    }

    /// Is this strike drone's assigned target currently radiating (`docs/DESIGN.md`
    /// §12.3)?
    ///
    /// True only for a **named air-defence battery** that is alive, has an organic radar,
    /// and is using it. Everything else is `false`, which is the honest answer rather than
    /// a permissive one: a unit, a command post or a bare map point emits nothing an ARM
    /// could ride, so an ARM aimed at one is flying blind by definition.
    ///
    /// `emitting` is therefore the counter, and it costs the radar: a battery under EMCON
    /// detects nothing through it, so it cannot cue itself and contributes no coverage.
    /// Survive the missile, or see the raid coming — not both.
    ///
    /// Deliberately **not** `self_cue`, which the two used to share. `self_cue` says who a
    /// battery listens to; sharing one flag let it take the missile protection of going
    /// dark while its radar carried on seeing everything (§12.5, V69).
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
    /// scenario, so one namespace is enough — and it means naming a SAM or a command post
    /// as a strike target simply works, which is what makes SEAD expressible in a
    /// scenario file rather than needing new syntax (`docs/DESIGN.md` §12).
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
