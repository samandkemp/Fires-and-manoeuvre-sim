//! What a burst does to the ground (`docs/DESIGN.md` §2.3, §12).
//!
//! Batteries, posts and units take the **same** Carleton kernel — they are all vehicles
//! sitting on the ground, and nothing about the maths cares which list they live in. That
//! is what made air defence SEAD-able rather than immortal.

use crate::fires;
use crate::fires::WeaponType;
use crate::sim::{Side, Sim};
use glam::Vec2;
use rand::Rng;

impl Sim {
    /// Apply one burst's area damage to every live enemy ground asset near it. Damage is
    /// the §2.3 Carleton kernel scaled by terrain cover, rolled per surviving element;
    /// rounds landing inside the suppression radius also suppress a **unit** (§4.3).
    ///
    /// Assets beyond `3·R_L` are skipped: the kernel is below 1.2e-4 there, so the cutoff
    /// keeps the sweep `O(assets)` without changing the model in any observable way.
    ///
    /// Batteries and posts take the same kernel as a unit — they are vehicles sitting on
    /// the ground, and nothing about the maths cares which list they live in. That is what
    /// makes them SEAD-able (§12); before it, a battery was simply immortal.
    ///
    /// **The three lists are swept in a fixed order** (batteries, posts, units) because
    /// each roll draws from the shared stream: reordering them would re-baseline every
    /// scenario fielding more than one kind of ground asset.
    pub(super) fn apply_area_damage(
        &mut self,
        burst: Vec2,
        weapon: &WeaponType,
        shooter_side: Side,
    ) -> u32 {
        let cutoff = 3.0 * weapon.lethal_radius_m;
        let mut casualties = 0u32;

        for i in 0..self.air_defence.len() {
            let (alive, side, pos, elements) = {
                let ad = &self.air_defence[i];
                (ad.alive(), ad.side, ad.pos, ad.elements)
            };
            if !alive || side == shooter_side {
                continue;
            }
            let Some(killed) = self.roll_area_damage(burst, weapon, cutoff, pos, elements) else {
                continue;
            };
            self.air_defence[i].elements = elements.saturating_sub(killed);
            casualties += killed;
            // A destroyed battery stops engaging: drop what it was holding, so its channels
            // are not left occupied by a corpse.
            if !self.air_defence[i].alive() {
                self.air_defence[i].engagements.clear();
            }
        }

        // A post shoots nothing, so killing it costs the defender no firepower at all —
        // only the coordination (§11).
        for i in 0..self.c2.len() {
            let (alive, side, pos, elements) = {
                let post = &self.c2[i];
                (post.alive(), post.side, post.pos, post.elements)
            };
            if !alive || side == shooter_side {
                continue;
            }
            let Some(killed) = self.roll_area_damage(burst, weapon, cutoff, pos, elements) else {
                continue;
            };
            self.c2[i].elements = elements.saturating_sub(killed);
            casualties += killed;
        }

        for i in 0..self.units.len() {
            let (alive, side, pos, elements) = {
                let u = &self.units[i];
                (u.alive(), u.side, u.pos, u.elements)
            };
            if !alive || side == shooter_side {
                continue;
            }
            let Some(killed) = self.roll_area_damage(burst, weapon, cutoff, pos, elements) else {
                continue;
            };
            self.units[i].elements = elements.saturating_sub(killed);
            casualties += killed;
            // Only a *unit* has a suppression state to be shaken; a near miss that did not
            // finish it puts its head down (§4.3).
            let miss = pos.distance(burst);
            if miss < self.suppression_radius_m
                && self.units[i].alive()
                && self.rng.random::<f32>() < self.p_suppress
            {
                self.units[i].suppression = self.units[i].suppression.step_up();
            }
        }
        casualties
    }

    /// Roll one burst against one ground asset: `None` if it is beyond the cutoff (no draw
    /// taken), else the number of its elements destroyed.
    ///
    /// The one place the §2.3 kernel meets the dice, shared by all three asset lists. They
    /// had three copies of this loop, which had already begun to drift — only the unit copy
    /// applied suppression, correctly, but as an absence rather than a decision.
    ///
    /// **One draw per surviving element, in element order**, exactly as before: the caller
    /// still visits the lists in the order `apply_area_damage` documents, so the RNG stream
    /// is untouched.
    fn roll_area_damage(
        &mut self,
        burst: Vec2,
        weapon: &WeaponType,
        cutoff: f32,
        pos: Vec2,
        elements: u32,
    ) -> Option<u32> {
        let miss = pos.distance(burst);
        if miss > cutoff {
            return None;
        }
        let cover = self.cover_at(pos);
        let damage = fires::carleton_damage(miss, weapon.lethal_radius_m) * (1.0 - cover);
        let mut killed = 0u32;
        for _ in 0..elements {
            if self.rng.random::<f32>() < damage {
                killed += 1;
            }
        }
        Some(killed)
    }
}
