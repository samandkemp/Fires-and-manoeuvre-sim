//! Ground fires: who shoots whom, and what one epoch of shooting does.
//! Spec: `docs/DESIGN.md` §2, §4.1–§4.2. Gates: V19–V24, V30, V31.
//!
//! One epoch, per shooter:
//!
//! 1. [`Sim::pick_target`] — nearest eligible enemy. Direct fire needs LOS and range;
//!    indirect needs a live *track* and range, but no LOS (it arcs over).
//! 2. Work out the shot once — range, hit probability, dispersion — since none of it
//!    varies across the burst.
//! 3. Fire `rof × epoch × elements` rounds, each resolving against the target.
//! 4. Apply suppression from the near-misses afterwards, in a fixed unit order.
//!
//! Fire volume scaling with *live elements* is what makes an aimed-fire duel obey
//! Lanchester's square law (V30): lose half your strength and you lose half your output.

use super::{FireEvent, Side, Sim};
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
    /// One epoch of fires. Each live, unpinned, weapon-carrying unit engages a target.
    pub(super) fn resolve_fires(&mut self) {
        // Reused across epochs so a battle does not allocate per epoch.
        let mut near_misses = std::mem::take(&mut self.near_misses);
        near_misses.clear();
        near_misses.resize(self.units.len(), 0);

        for s_idx in 0..self.units.len() {
            let shooter = &self.units[s_idx];
            if !shooter.alive() {
                continue;
            }
            let effectiveness = shooter
                .suppression
                .fire_effectiveness(self.suppressed_fire_factor);
            if effectiveness <= 0.0 {
                continue; // Pinned: no fire
            }
            let Some(weapon) = shooter.weapon.clone() else {
                continue;
            };
            let (shooter_side, shooter_pos, elements) =
                (shooter.side, shooter.pos, shooter.elements);

            let Some(t_idx) = self.pick_target(shooter_side, shooter_pos, &weapon) else {
                continue;
            };

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

    /// The target a shooter engages this epoch, or `None`. Direct fire: nearest live
    /// enemy in clear LOS and range. Indirect fire: nearest live *tracked* enemy in
    /// range (LOS not required — the round arcs over what blocks sight).
    ///
    /// Range is checked before LOS deliberately: the range test is a couple of terrain
    /// samples, an LOS traversal is a walk across the grid, so gating first keeps the
    /// expensive call off every out-of-range candidate.
    fn pick_target(
        &self,
        shooter_side: Side,
        shooter_pos: Vec2,
        weapon: &WeaponType,
    ) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, u) in self.units.iter().enumerate() {
            if u.side == shooter_side || !u.alive() {
                continue;
            }
            // Indirect fire needs a track; direct fire shoots what it can see. Checked
            // before the range computation because it is a bool, not geometry.
            if weapon.class == WeaponClass::Indirect && !u.detected {
                continue;
            }
            // Slant range (docs/DESIGN.md §9.1) — the one range convention.
            let r = los::slant_range(
                &self.terrain,
                shooter_pos,
                SHOOTER_HEIGHT_M,
                u.pos,
                u.stats.height_m,
            );
            if r > weapon.max_range_m || r < weapon.min_range_m {
                continue;
            }
            // Nothing further out than the incumbent can win, so skip the LOS walk.
            if best.is_some_and(|(_, br)| r >= br) {
                continue;
            }
            if weapon.class == WeaponClass::Direct
                && !los::visible(
                    &self.terrain,
                    shooter_pos,
                    SHOOTER_HEIGHT_M,
                    u.pos,
                    u.stats.height_m,
                )
            {
                continue;
            }
            best = Some((i, r));
        }
        best.map(|(i, _)| i)
    }

    /// Terrain cover `∈ [0, 1]` at a world position (0 outside the grid).
    pub(super) fn cover_at(&self, pos: Vec2) -> f32 {
        match self.terrain.transform().world_to_cell(pos) {
            Some((ix, iy)) => self.terrain.cover()[[iy, ix]],
            None => 0.0,
        }
    }
}
