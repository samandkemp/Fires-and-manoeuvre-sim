//! Ground fires: who shoots whom, and what one epoch of shooting does.
//! Spec: `docs/DESIGN.md` §2, §4.1–§4.2, §10.2. Gates: V19–V24, V30, V31, V56.
//!
//! One epoch:
//!
//! 1. [`Sim::allocate_fires`] — each side assigns **all** its shooters at once, by
//!    solving a weapon–target assignment problem (§10.2). Direct fire needs LOS and
//!    range; indirect needs a live *track* and range, but no LOS (it arcs over).
//! 2. Work out each shot once — range, hit probability, dispersion — since none of it
//!    varies across the burst.
//! 3. Fire `rof × epoch × elements` rounds, each resolving against the target.
//! 4. Apply suppression from the near-misses afterwards, in a fixed unit order.
//!
//! Fire volume scaling with *live elements* is what makes an aimed-fire duel obey
//! Lanchester's square law (V30): lose half your strength and you lose half your output.
//!
//! Allocation is deterministic and draws no randomness; only the rounds roll dice, and
//! they still roll in shooter index order. That is what keeps the draw stream comparable
//! with the pre-allocation engine (V58).

use super::{FireEvent, Side, Sim};
use crate::allocation::{self, Solver};
use crate::fires::{self, WeaponClass, WeaponType};
use crate::los;
use glam::Vec2;
use rand::Rng;

/// Firing height of a ground shooter above its own ground, metres — the sightline and
/// slant-range origin for direct fire.
pub(super) const SHOOTER_HEIGHT_M: f32 = 2.0;

/// Everything about a burst that does not change from round to round.
///
/// Range, hit probability and dispersion depend only on the shooter, the target and the
/// weapon, all fixed for the epoch — so they are computed once here rather than per
/// round. Only the dice differ between rounds.
enum Shot {
    /// Direct fire: a per-round kill probability against one element.
    Direct { p_kill: f32 },
    /// Indirect fire: a burst dispersion, plus the two damage modifiers.
    ///
    /// `cover` and `effectiveness` are kept as separate fields rather than pre-multiplied
    /// into one factor. Float multiplication is not associative, so folding them would
    /// give `d·(c·e)` where the original computed `(d·c)·e` — a difference of one ulp,
    /// but enough to flip a knife-edge kill roll and silently re-baseline V22/V24.
    Indirect {
        sigma_m: f32,
        lethal_radius_m: f32,
        cover: f32,
        effectiveness: f32,
    },
}

impl Sim {
    /// One epoch of fires. Each side allocates all its shooters, then they fire in index
    /// order.
    pub(super) fn resolve_fires(&mut self) {
        // Reused across epochs so a battle does not allocate per epoch.
        let mut near_misses = std::mem::take(&mut self.near_misses);
        near_misses.clear();
        near_misses.resize(self.units.len(), 0);

        // Both sides allocate against the same board, before anyone shoots — so neither
        // side gets to react to casualties the other has not taken yet. Sorted by shooter
        // so rounds still resolve in unit index order, which is the determinism unit.
        let mut orders: Vec<(usize, usize)> = [Side::Blue, Side::Red]
            .into_iter()
            .flat_map(|side| self.allocate_fires(side))
            .collect();
        orders.sort_unstable();

        for (s_idx, t_idx) in orders {
            let shooter = &self.units[s_idx];
            // The allocation was made against the board as it stood at the start of the
            // epoch; a shooter killed by an earlier shooter this epoch does not fire.
            if !shooter.alive() {
                continue;
            }
            let effectiveness = shooter
                .suppression
                .fire_effectiveness(self.suppressed_fire_factor);
            let Some(weapon) = shooter.weapon.clone() else {
                continue;
            };
            let (shooter_pos, elements) = (shooter.pos, shooter.elements);

            let shot = self.prepare_shot(&weapon, shooter_pos, t_idx, effectiveness);
            // Every live element fires the weapon's per-element round count.
            let per_element = (weapon.rof_rounds_per_min * self.epoch_s / 60.0).round() as u32;
            let rounds = per_element.saturating_mul(elements);
            let target_pos = self.units[t_idx].pos;

            let mut remaining = self.units[t_idx].elements;
            let mut casualties = 0u32;
            for _ in 0..rounds {
                if remaining == 0 {
                    break;
                }
                let (killed, near) = self.fire_one_round(&shot, target_pos, remaining);
                remaining -= killed;
                casualties += killed;
                near_misses[t_idx] += near;
            }

            if casualties > 0 {
                let target = &mut self.units[t_idx];
                let before = target.elements;
                target.elements = target.elements.saturating_sub(casualties);
                self.fire_events.push(FireEvent {
                    time_s: self.time_s,
                    shooter: s_idx,
                    target: t_idx,
                    casualties,
                    killed: before > 0 && target.elements == 0,
                });
            }
        }

        // Apply near-miss suppression after all firing (fixed order).
        for (u_idx, &count) in near_misses.iter().enumerate() {
            for _ in 0..count {
                if self.rng.random::<f32>() < self.p_suppress {
                    self.units[u_idx].suppression = self.units[u_idx].suppression.step_up();
                }
            }
        }

        self.near_misses = near_misses; // hand the buffer back
    }

    /// Everything about the burst that the dice do not change (§2.2, §2.3).
    fn prepare_shot(
        &self,
        weapon: &WeaponType,
        shooter_pos: Vec2,
        t_idx: usize,
        effectiveness: f32,
    ) -> Shot {
        let target = &self.units[t_idx];
        let cover = self.cover_at(target.pos);
        match weapon.class {
            WeaponClass::Direct => {
                // The round flies the slant distance, so dispersion scales with that.
                let range = los::slant_range(
                    &self.terrain,
                    shooter_pos,
                    SHOOTER_HEIGHT_M,
                    target.pos,
                    target.stats.height_m,
                );
                let p_hit = fires::direct_p_hit(
                    weapon.dispersion_mrad,
                    range,
                    target.stats.silhouette_width_m,
                    target.stats.height_m,
                );
                Shot::Direct {
                    p_kill: p_hit * weapon.p_kill_given_hit * (1.0 - cover) * effectiveness,
                }
            }
            WeaponClass::Indirect => Shot::Indirect {
                sigma_m: fires::sigma_from_cep(weapon.cep_m),
                lethal_radius_m: weapon.lethal_radius_m,
                cover,
                effectiveness,
            },
        }
    }

    /// Resolve one round: returns `(elements_killed, near_miss)`. `remaining` is the
    /// target's live element count before this round — indirect fire rolls each one.
    fn fire_one_round(&mut self, shot: &Shot, target_pos: Vec2, remaining: u32) -> (u32, u32) {
        match *shot {
            Shot::Direct { p_kill } => {
                if self.rng.random::<f32>() < p_kill {
                    (1, 0) // element destroyed
                } else {
                    (0, 1) // round passed close — a near-miss
                }
            }
            Shot::Indirect {
                sigma_m,
                lethal_radius_m,
                cover,
                effectiveness,
            } => {
                let burst = fires::sample_burst(target_pos, sigma_m, &mut self.rng);
                let miss = burst.distance(target_pos);
                // Same multiplication order as before the hoist — see `Shot::Indirect`.
                let dmg =
                    fires::carleton_damage(miss, lethal_radius_m) * (1.0 - cover) * effectiveness;
                // Each remaining element independently survives or not.
                let mut killed = 0u32;
                for _ in 0..remaining {
                    if self.rng.random::<f32>() < dmg {
                        killed += 1;
                    }
                }
                (killed, u32::from(miss < self.suppression_radius_m))
            }
        }
    }

    /// Assign every one of `side`'s shooters to an enemy, as `(shooter, target)` pairs.
    /// `docs/DESIGN.md` §10.2.
    ///
    /// The old rule was "each shooter takes the nearest enemy it can engage", decided
    /// independently. That wastes fire in the obvious way: three tanks all engage the one
    /// nearest target while a second, equally dangerous one is left alone. Here the side
    /// solves for all its shooters at once.
    ///
    /// Deterministic, and draws no randomness.
    fn allocate_fires(&self, side: Side) -> Vec<(usize, usize)> {
        let shooters: Vec<usize> = (0..self.units.len())
            .filter(|&i| {
                let u = &self.units[i];
                u.side == side
                    && u.alive()
                    && u.weapon.is_some()
                    // Pinned units emit nothing (V31), so they are not in the problem.
                    && u.suppression
                        .fire_effectiveness(self.suppressed_fire_factor)
                        > 0.0
            })
            .collect();
        let targets: Vec<usize> = (0..self.units.len())
            .filter(|&i| self.units[i].side != side && self.units[i].alive())
            .collect();
        if shooters.is_empty() || targets.is_empty() {
            return Vec::new();
        }

        // What each shooter would do to each target, as a fraction of the target
        // destroyed this epoch.
        let kill_fraction: Vec<Vec<Option<f64>>> = shooters
            .iter()
            .map(|&s| targets.iter().map(|&t| self.kill_fraction(s, t)).collect())
            .collect();

        let value_scale = self.threat_scale();
        // Slots: one per remaining element, capped. Slot k of a target is worth less than
        // slot k-1 — see `slot_weight`.
        let mut slot_target = Vec::new();
        let mut slot_weight = Vec::new();
        for (t_pos, &t) in targets.iter().enumerate() {
            let elements = self.units[t].elements;
            let count = elements.min(self.max_shooters_per_target).max(1);
            // A representative kill probability for this target, over the shooters that
            // can actually engage it. Exact when they are identical, which is the case
            // the geometric discount below is derived for.
            let engaging: Vec<f64> = kill_fraction.iter().filter_map(|row| row[t_pos]).collect();
            let q_bar = if engaging.is_empty() {
                0.0
            } else {
                engaging.iter().sum::<f64>() / engaging.len() as f64
            };
            let value = f64::from(self.target_value(t, value_scale));
            for k in 0..count {
                slot_target.push(t_pos);
                // Geometric discount: the (k+1)-th shooter only helps if the k before it
                // all failed, which happens with probability (1 - q)^k. This is the
                // standard weapon-target-assignment decomposition, and it is exact when
                // the shooters on a target are alike.
                slot_weight.push(value * (1.0 - q_bar).powi(k as i32));
            }
        }

        let payoff: Vec<Vec<f64>> = kill_fraction
            .iter()
            .map(|row| {
                slot_target
                    .iter()
                    .zip(&slot_weight)
                    .map(|(&t_pos, &weight)| match row[t_pos] {
                        Some(q) if q > 0.0 => q * weight,
                        _ => allocation::ineligible(),
                    })
                    .collect()
            })
            .collect();

        let solver: Solver = self.allocation.into();
        allocation::solve(&payoff, solver)
            .into_iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.map(|j| (shooters[i], targets[slot_target[j]])))
            .collect()
    }

    /// The fraction of target `t` that shooter `s` expects to destroy this epoch, or
    /// `None` if it cannot engage at all.
    ///
    /// Direct fire needs LOS and range; indirect fire needs a live track and range but no
    /// LOS. Range is checked before LOS deliberately: the range test is a couple of
    /// terrain samples, an LOS traversal walks the grid.
    fn kill_fraction(&self, s: usize, t: usize) -> Option<f64> {
        let shooter = &self.units[s];
        let target = &self.units[t];
        let weapon = shooter.weapon.as_ref()?;

        if weapon.class == WeaponClass::Indirect && !target.detected {
            return None;
        }
        // Slant range (docs/DESIGN.md §9.1) — the one range convention.
        let range = los::slant_range(
            &self.terrain,
            shooter.pos,
            SHOOTER_HEIGHT_M,
            target.pos,
            target.stats.height_m,
        );
        if range > weapon.max_range_m || range < weapon.min_range_m {
            return None;
        }

        let effectiveness = shooter
            .suppression
            .fire_effectiveness(self.suppressed_fire_factor);
        let cover = self.cover_at(target.pos);
        let rounds = f64::from(self.rounds_this_epoch(weapon, shooter.elements));
        let elements = f64::from(target.elements.max(1));

        let fraction = match weapon.class {
            WeaponClass::Direct => {
                if !los::visible(
                    &self.terrain,
                    shooter.pos,
                    SHOOTER_HEIGHT_M,
                    target.pos,
                    target.stats.height_m,
                ) {
                    return None;
                }
                let p_hit = fires::direct_p_hit(
                    weapon.dispersion_mrad,
                    range,
                    target.stats.silhouette_width_m,
                    target.stats.height_m,
                );
                // Each round removes at most one element.
                let p_kill =
                    f64::from(p_hit * weapon.p_kill_given_hit * (1.0 - cover) * effectiveness);
                rounds * p_kill / elements
            }
            WeaponClass::Indirect => {
                // Each round rolls every surviving element, so the expected fraction
                // removed per round is just the expected damage.
                let sigma = fires::sigma_from_cep(weapon.cep_m);
                let per_round = fires::expected_area_damage(0.0, sigma, weapon.lethal_radius_m)
                    * (1.0 - cover)
                    * effectiveness;
                rounds * f64::from(per_round)
            }
        };
        Some(fraction.clamp(0.0, 1.0))
    }

    /// Rounds one unit fires in an epoch: the weapon's rate, times its live elements.
    fn rounds_this_epoch(&self, weapon: &WeaponType, elements: u32) -> u32 {
        let per_element = (weapon.rof_rounds_per_min * self.epoch_s / 60.0).round() as u32;
        per_element.saturating_mul(elements)
    }

    /// The largest raw threat score on the field, used to normalise [`Sim::target_value`].
    /// Zero when nobody is armed, in which case value is size alone.
    fn threat_scale(&self) -> f32 {
        self.units
            .iter()
            .filter(|u| u.alive())
            .map(Self::raw_threat)
            .fold(0.0f32, f32::max)
    }

    /// How dangerous a unit is, before normalisation: rate of fire × lethality × reach.
    /// Unarmed units score zero.
    fn raw_threat(unit: &super::UnitState) -> f32 {
        unit.weapon.as_ref().map_or(0.0, |w| {
            w.rof_rounds_per_min * w.p_kill_given_hit.max(0.01) * w.max_range_m
        })
    }

    /// What destroying target `t` is worth (`docs/DESIGN.md` §10.2).
    ///
    /// The `value` dial on the stat block wins when set. Otherwise it is derived:
    /// `elements × (1 + threat/threat_max)`, so a unit is worth its size, doubled if it is
    /// the most dangerous thing on the field. Deriving from size *and* threat means an
    /// unscored stat block still ranks sensibly — an unarmed truck is worth something, a
    /// full-strength gun battery a great deal more.
    fn target_value(&self, t: usize, threat_scale: f32) -> f32 {
        let unit = &self.units[t];
        let per_element = unit.stats.value.unwrap_or_else(|| {
            if threat_scale > 0.0 {
                1.0 + Self::raw_threat(unit) / threat_scale
            } else {
                1.0
            }
        });
        unit.elements as f32 * per_element.max(0.0)
    }

    /// Terrain cover `∈ [0, 1]` at a world position (0 outside the grid).
    pub(super) fn cover_at(&self, pos: Vec2) -> f32 {
        match self.terrain.transform().world_to_cell(pos) {
            Some((ix, iy)) => self.terrain.cover()[[iy, ix]],
            None => 0.0,
        }
    }
}
