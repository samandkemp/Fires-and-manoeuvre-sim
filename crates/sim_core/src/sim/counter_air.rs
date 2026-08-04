//! The air phases of the tick (`docs/DESIGN.md` §9): detecting airborne targets,
//! air-defence engagement, and strike release.
//!
//! These are appended to the tick per §9.6 and draw no RNG at all when the air and
//! air-defence lists are empty, which is what keeps a drone-free scenario bit-identical
//! to the pre-air engine (V52).
//!
//! A child module of `sim`, so it still reaches `Sim`'s private fields — splitting the
//! file cost nothing in encapsulation.

use super::{AirDefenceEvent, AirDetectionEvent, GlimpseTarget, Side, Sim, StrikeEvent};
use crate::air::TargetSpec;
use crate::fires::WeaponType;
use crate::{air_defence, fires, los};
use glam::Vec2;
use rand::Rng;

impl Sim {
    /// The glimpse process against airborne targets (§9.1). Same as the ground loop
    /// except the target's actor height comes from its altitude and it contributes no
    /// terrain concealment — it isn't standing in the cell below it. Canopy transmittance
    /// still applies, being a property of the sightline rather than the target.
    pub(super) fn detect_air(&mut self) {
        if self.air.is_empty() {
            return;
        }
        for s_idx in 0..self.sensors.len() {
            if !self.sensor_active(s_idx) {
                continue;
            }
            let view = self.sensor_view(s_idx);
            for a_idx in 0..self.air.len() {
                let (sensor, air) = (&self.sensors[s_idx], &self.air[a_idx]);
                // Gated per *sensor*, not on the global `detected` flag the ground loop
                // uses: air defence needs to know when each battery's own radar saw the
                // target, so every sensor keeps glimpsing until it has (§9.5).
                if air.side == sensor.side || !air.alive || air.seen_by.contains_key(&s_idx) {
                    continue;
                }
                let target = GlimpseTarget {
                    kind: super::los_cache::TargetKind::Air,
                    idx: a_idx,
                    pos: air.pos,
                    height_m: air.actor_height(&self.terrain),
                    signature: air.stats.signature_in(sensor.stats.modality),
                    // Airborne: not standing in the cell below it, so no concealment.
                    // Canopy transmittance still applies — that is the sightline's.
                    concealment: 0.0,
                    side: air.side,
                };
                if self.glimpse(s_idx, view, target) {
                    let air = &mut self.air[a_idx];
                    air.seen_by.insert(s_idx, self.time_s);
                    air.last_seen_s = Some(self.time_s);
                    let first = !air.detected;
                    if first {
                        air.detected = true;
                        air.detected_at_s = Some(self.time_s);
                        air.detected_by = Some(s_idx);
                    }
                    let air_pos = air.pos;
                    // Log only the first detection, so the feed stays a track list
                    // rather than one line per sensor that later acquires it.
                    if first {
                        self.air_events.push(AirDetectionEvent {
                            time_s: self.time_s,
                            sensor: s_idx,
                            air: a_idx,
                            air_pos,
                        });
                    }
                }
            }
        }
    }

    /// One tick of air defence (§9.4–§9.5): drop engagements whose target died or left
    /// the envelope, resolve shots that are due, then open new ones on the nearest
    /// actionable targets while channels and magazine last. Fixed index order throughout,
    /// which is the determinism unit.
    pub(super) fn resolve_air_defence(&mut self) {
        if self.air_defence.is_empty() || self.air.is_empty() {
            return;
        }
        let (now, dt) = (self.time_s, self.dt_s);
        // Each airframe's actor height, computed once and shared by every battery.
        let heights: Vec<f32> = self
            .air
            .iter()
            .map(|a| a.actor_height(&self.terrain))
            .collect();

        let mut resolutions = Vec::new();
        for ad_idx in 0..self.air_defence.len() {
            // Which targets this battery may engage right now, and at what range.
            let mut engageable = vec![false; self.air.len()];
            let mut ranges = vec![0.0f32; self.air.len()];
            {
                let ad = &self.air_defence[ad_idx];
                for (a_idx, air) in self.air.iter().enumerate() {
                    if !air.alive || air.side == ad.side {
                        continue;
                    }
                    // A track must have arrived *and* aged through the cueing timeline —
                    // by whichever route reaches this battery first (§9.5).
                    if !ad
                        .actionable_at(air.detected_at_s, ad.own_sensor_seen(&air.seen_by))
                        .is_some_and(|t| now >= t)
                    {
                        continue;
                    }
                    // One call answers both "can I engage?" and "at what range?".
                    let Some(range) = air_defence::engagement_range(
                        &ad.stats,
                        &self.terrain,
                        ad.pos,
                        air.pos,
                        heights[a_idx],
                    ) else {
                        continue;
                    };
                    engageable[a_idx] = true;
                    ranges[a_idx] = range;
                }
            }

            resolutions.clear();
            {
                // Borrow the two fields separately — `self.air_defence` and `self.rng`
                // are disjoint, which the borrow checker accepts as long as we reach
                // them as fields rather than through a `&mut self` method.
                let ad = &mut self.air_defence[ad_idx];
                ad.drop_engagements(|t| engageable.get(t).copied().unwrap_or(false));
                ad.resolve_due(now, dt, &mut self.rng, &mut resolutions);
            }
            for &(target, killed) in &resolutions {
                if killed {
                    self.air[target].alive = false;
                }
                self.air_defence_events.push(AirDefenceEvent {
                    time_s: now,
                    battery: ad_idx,
                    air: target,
                    killed,
                });
            }

            // Commit new engagements, nearest first; ties break on index so the order is
            // deterministic.
            let mut candidates: Vec<(usize, f32)> = engageable
                .iter()
                .enumerate()
                .filter(|(_, &e)| e)
                .map(|(i, _)| (i, ranges[i]))
                .collect();
            candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            for (a_idx, range) in candidates {
                if !self.air[a_idx].alive {
                    continue;
                }
                let ad = &mut self.air_defence[ad_idx];
                if !ad.can_open(now) {
                    break;
                }
                if ad.engaging(a_idx) {
                    continue;
                }
                ad.open(a_idx, now, range);
            }
        }
    }

    /// Strike release (`docs/DESIGN.md` §9.3): a drone within `release_range_m` of its
    /// aim point drops one munition, which resolves exactly as a §2.3 indirect round.
    pub(super) fn resolve_strikes(&mut self) {
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
            let sigma = fires::sigma_from_cep(weapon.cep_m);
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
    fn strike_aim_point(&self, air_idx: usize) -> Option<Vec2> {
        match &self.air[air_idx].target {
            Some(TargetSpec::Point(p)) => Some(*p),
            Some(TargetSpec::Unit(id)) => self
                .units
                .iter()
                .find(|u| &u.id == id && u.alive())
                .map(|u| u.pos),
            None => self.air[air_idx].plan.destination(),
        }
    }

    /// Apply one burst's area damage to every live enemy ground unit near it. Damage is
    /// the §2.3 Carleton kernel scaled by terrain cover, rolled per surviving element;
    /// rounds landing inside the suppression radius also suppress (§4.3).
    ///
    /// Units beyond `3·R_L` are skipped: the kernel is below 1.2e-4 there, so the cutoff
    /// keeps the sweep `O(units)` without changing the model in any observable way.
    fn apply_area_damage(&mut self, burst: Vec2, weapon: &WeaponType, shooter_side: Side) -> u32 {
        let cutoff = 3.0 * weapon.lethal_radius_m;
        let mut casualties = 0u32;
        for u_idx in 0..self.units.len() {
            let target = &self.units[u_idx];
            if !target.alive() || target.side == shooter_side {
                continue;
            }
            let miss = target.pos.distance(burst);
            if miss > cutoff {
                continue;
            }
            let cover = self.cover_at(target.pos);
            let damage = fires::carleton_damage(miss, weapon.lethal_radius_m) * (1.0 - cover);
            let remaining = self.units[u_idx].elements;
            let mut killed = 0u32;
            for _ in 0..remaining {
                if self.rng.random::<f32>() < damage {
                    killed += 1;
                }
            }
            if killed > 0 {
                self.units[u_idx].elements = remaining.saturating_sub(killed);
                casualties += killed;
            }
            // A near-miss that did not finish the unit suppresses it.
            if miss < self.suppression_radius_m
                && self.units[u_idx].alive()
                && self.rng.random::<f32>() < self.p_suppress
            {
                self.units[u_idx].suppression = self.units[u_idx].suppression.step_up();
            }
        }
        casualties
    }
}
