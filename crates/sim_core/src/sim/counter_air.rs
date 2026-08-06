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
use crate::allocation::{self, Solver};
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
        // Batteries under C2 defer their opening to one coordinated pass (§11).
        let mut coordinated: Vec<(usize, Vec<bool>, Vec<f32>)> = Vec::new();
        for ad_idx in 0..self.air_defence.len() {
            // A destroyed battery does nothing at all (§12).
            if !self.air_defence[ad_idx].alive() {
                continue;
            }
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

            // A battery under a live friendly C2 post defers its opening to the
            // coordinated pass below (§11). One that is not opens for itself, right here,
            // by exactly the rule it always used — which is what keeps a scenario with no
            // C2 post bit-identical to the pre-C2 engine (V59).
            if self.coordinated(ad_idx) {
                coordinated.push((ad_idx, engageable, ranges));
                continue;
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

        self.open_coordinated(&coordinated, now);
    }

    /// Is this battery under a live friendly C2 post (`docs/DESIGN.md` §11)?
    fn coordinated(&self, ad_idx: usize) -> bool {
        let ad = &self.air_defence[ad_idx];
        self.c2
            .iter()
            .any(|post| post.side == ad.side && post.covers(ad.pos))
    }

    /// Open engagements for every C2-coordinated battery at once, so the group splits the
    /// raid instead of each battery independently taking whatever is nearest.
    ///
    /// **Each free channel is a row.** A two-channel CIWS contributes two rows, so
    /// `channels` falls out of the assignment structure rather than needing a special
    /// case — and a battery with none contributes nothing, which is exactly right.
    ///
    /// Draws no randomness: this decides *who shoots at what*, and the shooting itself is
    /// resolved by `resolve_due` on a later tick, in battery index order as before.
    fn open_coordinated(&mut self, coordinated: &[(usize, Vec<bool>, Vec<f32>)], now: f64) {
        if coordinated.is_empty() {
            return;
        }
        // Rows: one per free channel. `can_open` already folds in magazine and readiness.
        let mut rows: Vec<(usize, f32)> = Vec::new(); // (battery, its range row is scored on)
        for &(ad_idx, _, _) in coordinated {
            let ad = &self.air_defence[ad_idx];
            if !ad.can_open(now) {
                continue;
            }
            let free = (ad.stats.channels as usize).saturating_sub(ad.engagements.len());
            let free = free.min(ad.magazine_left.max(1) as usize);
            for _ in 0..free {
                rows.push((ad_idx, 0.0));
            }
        }
        if rows.is_empty() {
            return;
        }

        // Columns: slots on each airframe any coordinated battery can engage.
        let mut targets: Vec<usize> = Vec::new();
        for (_, engageable, _) in coordinated {
            for (a_idx, &ok) in engageable.iter().enumerate() {
                if ok && self.air[a_idx].alive && !targets.contains(&a_idx) {
                    targets.push(a_idx);
                }
            }
        }
        if targets.is_empty() {
            return;
        }
        targets.sort_unstable();

        let cap = self.max_shooters_per_target.max(1) as usize;
        let mut slot_target = Vec::new();
        let mut slot_rank = Vec::new();
        for (t_pos, _) in targets.iter().enumerate() {
            for k in 0..cap {
                slot_target.push(t_pos);
                slot_rank.push(k);
            }
        }

        // Which batteries can reach which airframe, and at what range.
        let engageable_by: Vec<&Vec<bool>> = coordinated.iter().map(|(_, e, _)| e).collect();
        let ranges_by: Vec<&Vec<f32>> = coordinated.iter().map(|(_, _, r)| r).collect();
        let index_of = |ad_idx: usize| {
            coordinated
                .iter()
                .position(|&(i, _, _)| i == ad_idx)
                .expect("row batteries come from the coordinated list")
        };

        let payoff: Vec<Vec<f64>> = rows
            .iter()
            .map(|&(ad_idx, _)| {
                let g = index_of(ad_idx);
                slot_target
                    .iter()
                    .zip(&slot_rank)
                    .map(|(&t_pos, &k)| {
                        let a_idx = targets[t_pos];
                        if !engageable_by[g][a_idx] || !self.air[a_idx].alive {
                            return allocation::ineligible();
                        }
                        let p = self.p_kill_before_release(ad_idx, a_idx, ranges_by[g][a_idx]);
                        if p <= 0.0 {
                            return allocation::ineligible();
                        }
                        let value = f64::from(
                            self.air[a_idx]
                                .stats
                                .threat_value(self.air[a_idx].munitions_left),
                        );
                        // Same geometric discount as ground fires (§10.2): a second
                        // battery on one drone only helps if the first misses.
                        f64::from(p) * value * f64::from(1.0 - p).powi(k as i32)
                    })
                    .collect()
            })
            .collect();

        let solver: Solver = self.allocation.into();
        for (row, slot) in allocation::solve(&payoff, solver).into_iter().enumerate() {
            let Some(j) = slot else { continue };
            let (ad_idx, _) = rows[row];
            let a_idx = targets[slot_target[j]];
            if !self.air[a_idx].alive {
                continue;
            }
            let g = index_of(ad_idx);
            let range = ranges_by[g][a_idx];
            let ad = &mut self.air_defence[ad_idx];
            if !ad.can_open(now) || ad.engaging(a_idx) {
                continue;
            }
            ad.open(a_idx, now, range);
        }
    }

    /// Probability this battery destroys this airframe **before it releases** — the
    /// payoff air-defence allocation maximises (`docs/DESIGN.md` §11.2).
    ///
    /// The deadline that matters is the release point, not the envelope edge: a drone
    /// that leaves the envelope having already dropped its munition has won. So the
    /// window is the time until it reaches `release_range_m` of its aim point, and a
    /// battery is rewarded for intercepting the airframe that is closest to doing damage
    /// rather than the one that happens to be nearest.
    ///
    /// An airframe with nothing to drop has no such deadline; it is scored over the time
    /// it takes to cross the remaining envelope instead, so a recce drone is still worth
    /// shooting, just not urgently.
    fn p_kill_before_release(&self, ad_idx: usize, a_idx: usize, range_m: f32) -> f32 {
        let ad = &self.air_defence[ad_idx];
        let air = &self.air[a_idx];
        let speed = air.speed_m_s.max(1.0);

        let window = match (air.can_strike(), self.strike_aim_point(a_idx)) {
            (true, Some(aim)) => {
                let to_go = air.pos.distance(aim) - air.stats.release_range_m;
                (to_go / speed).max(0.0)
            }
            // No munition to drop: score it over the time to cross the envelope.
            _ => ((range_m - ad.stats.min_range_m) / speed).max(0.0),
        };
        if window <= 0.0 {
            return 0.0; // already at its release point; nothing to be gained
        }
        air_defence::p_kill_in_window(&ad.stats, range_m, window)
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
            Some(TargetSpec::Named(id)) => self.named_ground_asset(id),
            None => self.air[air_idx].plan.destination(),
        }
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

    /// Apply one burst's area damage to every live enemy ground unit near it. Damage is
    /// the §2.3 Carleton kernel scaled by terrain cover, rolled per surviving element;
    /// rounds landing inside the suppression radius also suppress (§4.3).
    ///
    /// Units beyond `3·R_L` are skipped: the kernel is below 1.2e-4 there, so the cutoff
    /// keeps the sweep `O(units)` without changing the model in any observable way.
    fn apply_area_damage(&mut self, burst: Vec2, weapon: &WeaponType, shooter_side: Side) -> u32 {
        let cutoff = 3.0 * weapon.lethal_radius_m;
        let mut casualties = 0u32;
        // Air defence and C2 take the same kernel as a unit: they are vehicles sitting on
        // the ground, and nothing about the maths cares which list they live in. This is
        // what makes them SEAD-able (§12) — before it, a battery was simply immortal.
        casualties += self.damage_air_defence(burst, weapon, shooter_side, cutoff);
        casualties += self.damage_c2(burst, weapon, shooter_side, cutoff);
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

    /// Area damage against enemy air-defence batteries. Destroying one silences it *and*
    /// its organic radar, which is handled by [`Sim::sensor_active`].
    fn damage_air_defence(
        &mut self,
        burst: Vec2,
        weapon: &WeaponType,
        shooter_side: Side,
        cutoff: f32,
    ) -> u32 {
        let mut killed_total = 0u32;
        for i in 0..self.air_defence.len() {
            let ad = &self.air_defence[i];
            if !ad.alive() || ad.side == shooter_side {
                continue;
            }
            let miss = ad.pos.distance(burst);
            if miss > cutoff {
                continue;
            }
            let cover = self.cover_at(self.air_defence[i].pos);
            let damage = fires::carleton_damage(miss, weapon.lethal_radius_m) * (1.0 - cover);
            let remaining = self.air_defence[i].elements;
            let mut killed = 0u32;
            for _ in 0..remaining {
                if self.rng.random::<f32>() < damage {
                    killed += 1;
                }
            }
            if killed > 0 {
                self.air_defence[i].elements = remaining.saturating_sub(killed);
                killed_total += killed;
                // A destroyed battery stops engaging: drop what it was holding so its
                // channels are not left occupied by a corpse.
                if !self.air_defence[i].alive() {
                    self.air_defence[i].engagements.clear();
                }
            }
        }
        killed_total
    }

    /// Area damage against enemy C2 posts. A post shoots nothing, so killing it costs the
    /// defender no firepower at all — only the coordination (§11).
    fn damage_c2(
        &mut self,
        burst: Vec2,
        weapon: &WeaponType,
        shooter_side: Side,
        cutoff: f32,
    ) -> u32 {
        let mut killed_total = 0u32;
        for i in 0..self.c2.len() {
            let post = &self.c2[i];
            if !post.alive() || post.side == shooter_side {
                continue;
            }
            let miss = post.pos.distance(burst);
            if miss > cutoff {
                continue;
            }
            let cover = self.cover_at(self.c2[i].pos);
            let damage = fires::carleton_damage(miss, weapon.lethal_radius_m) * (1.0 - cover);
            let remaining = self.c2[i].elements;
            let mut killed = 0u32;
            for _ in 0..remaining {
                if self.rng.random::<f32>() < damage {
                    killed += 1;
                }
            }
            if killed > 0 {
                self.c2[i].elements = remaining.saturating_sub(killed);
                killed_total += killed;
            }
        }
        killed_total
    }
}
