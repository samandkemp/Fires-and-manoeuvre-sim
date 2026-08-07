//! The coordinated air-defence assignment (`docs/DESIGN.md` §11.2).
//!
//! One assignment **per side**: rows are free engagement channels, columns are slots on
//! each engageable airframe, and the payoff is `P(kill before release) × value`. A post
//! coordinates its own batteries and nobody else's, which is why the side is a parameter
//! here rather than something read off the first battery in the list.

use super::{CoordinatedBattery, AD_PLANNING_HORIZON_S};
use crate::air_defence;
use crate::allocation::{self, Solver};
use crate::sim::{Side, Sim};

impl Sim {
    /// Open engagements for **one side's** C2-coordinated batteries at once, so the group
    /// splits the raid instead of each battery independently taking whatever is nearest.
    ///
    /// **Each free channel is a row.** A two-channel CIWS contributes two rows, so
    /// `channels` falls out of the assignment structure rather than needing a special
    /// case — and a battery with none contributes nothing, which is exactly right.
    ///
    /// **One side per call.** The side is passed in rather than read off the first battery:
    /// coordination is a relationship between a post and *its own* batteries, so a group
    /// spanning both sides has no single doctrine to be scored under, and no single
    /// `max_batteries_per_air_target` budget to spend.
    ///
    /// Draws no randomness: this decides *who shoots at what*, and the shooting itself is
    /// resolved by `resolve_due` on a later tick, in battery index order as before.
    pub(super) fn open_coordinated(
        &mut self,
        coordinated: &[CoordinatedBattery],
        side: Side,
        now: f64,
    ) {
        if coordinated.is_empty() {
            return;
        }
        debug_assert!(
            coordinated
                .iter()
                .all(|&(i, _, _)| self.air_defence[i].side == side),
            "a coordinated group holds one side's batteries only"
        );
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
        let mut all_targets: Vec<usize> = Vec::new();
        for (_, engageable, _) in coordinated {
            for (a_idx, &ok) in engageable.iter().enumerate() {
                if ok && self.air[a_idx].alive && !all_targets.contains(&a_idx) {
                    all_targets.push(a_idx);
                }
            }
        }
        if all_targets.is_empty() {
            return;
        }
        all_targets.sort_unstable();

        // Doctrine (§13.2). Without one this is a single group at weight 1 — exactly the
        // pre-doctrine solve. Strict runs the assignment once per tier, highest first, so
        // a free channel that can reach a priority airframe spends itself there before it
        // is offered anything lower.
        let (groups, weights) = self.air_target_groups(side, &all_targets);
        let mut free_rows = rows;
        for (targets, weight) in groups.iter().zip(&weights) {
            if free_rows.is_empty() {
                break;
            }
            let used = self.open_group(coordinated, side, &free_rows, targets, *weight, now);
            free_rows = free_rows
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !used.contains(i))
                .map(|(_, r)| r)
                .collect();
        }
    }

    /// Which doctrine tier an airframe sits in for `side`. Tier 0 for everything when the
    /// side has no doctrine, which is what makes the undirected path unchanged.
    pub(super) fn air_tier(&self, side: Side, a_idx: usize) -> usize {
        self.doctrine_of(side)
            .tier_of(&crate::doctrine::TargetNames {
                id: &self.air[a_idx].id,
                role: self.air[a_idx].stats.role.as_deref(),
                class: "air",
            })
    }

    /// Split the engageable airframes into doctrine tiers, with the value multiplier each
    /// group carries (`docs/DESIGN.md` §13.2).
    ///
    /// One group at weight 1 when the side has no doctrine, or when its mode is
    /// `Weighted` — in the weighted case the multipliers ride on the per-target value
    /// instead, so the solve stays a single problem and the payoff still decides.
    fn air_target_groups(&self, side: Side, targets: &[usize]) -> (Vec<Vec<usize>>, Vec<f32>) {
        let doc = self.doctrine_of(side);
        let tier_of = |a_idx: usize| {
            doc.tier_of(&crate::doctrine::TargetNames {
                id: &self.air[a_idx].id,
                role: self.air[a_idx].stats.role.as_deref(),
                class: "air",
            })
        };
        if doc.mode == crate::doctrine::DoctrineMode::Weighted {
            // A single solve; the tier only scales value. Handled by `open_group` reading
            // the weight per target, so hand it the whole set and a sentinel weight.
            return (vec![targets.to_vec()], vec![f32::NEG_INFINITY]);
        }
        let mut groups = Vec::new();
        for tier in 0..doc.tier_count() {
            let g: Vec<usize> = targets
                .iter()
                .copied()
                .filter(|&a| tier_of(a) == tier)
                .collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
        let weights = vec![1.0; groups.len()];
        (groups, weights)
    }

    /// Solve one assignment over `targets` and open the engagements it chose, returning
    /// which rows were consumed.
    ///
    /// `weight` scales every target's value; the sentinel `-inf` means "look the weight up
    /// per target from doctrine", which is how `Weighted` mode gets its per-tier scaling
    /// without a second solve.
    #[allow(clippy::too_many_lines)]
    fn open_group(
        &mut self,
        coordinated: &[CoordinatedBattery],
        side: Side,
        rows: &[(usize, f32)],
        targets: &[usize],
        weight: f32,
        now: f64,
    ) -> Vec<usize> {
        if rows.is_empty() || targets.is_empty() {
            return Vec::new();
        }
        let doc = self.doctrine_of(side);
        let weight_of = |a_idx: usize| -> f32 {
            if weight.is_finite() {
                return weight;
            }
            doc.weight_for_tier(doc.tier_of(&crate::doctrine::TargetNames {
                id: &self.air[a_idx].id,
                role: self.air[a_idx].stats.role.as_deref(),
                class: "air",
            }))
        };

        let cap = self.max_batteries_per_air_target.max(1) as usize;
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
                                .threat_value(self.air[a_idx].munitions_left)
                                * weight_of(a_idx),
                        );
                        // Same geometric discount as ground fires (§10.2): a second
                        // battery on one drone only helps if the first misses.
                        f64::from(p) * value * f64::from(1.0 - p).powi(k as i32)
                    })
                    .collect()
            })
            .collect();

        let solver: Solver = self.allocation.into();
        let mut used = Vec::new();
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
            used.push(row);
        }
        used
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
        }
        // Capped at a planning horizon, for two reasons. Beyond about a minute, "how long
        // this target will linger" stops discriminating usefully — the defence will have
        // reconsidered many times. More concretely, an uncapped window runs to hundreds of
        // seconds for a distant loiterer, which drives `p_kill` to 1 for *every* pairing;
        // the diminishing-return discount `(1 - p)^k` then collapses to 1 and stops
        // separating "cover another drone" from "pile onto this one", which is the whole
        // job it is there to do.
        .min(AD_PLANNING_HORIZON_S);
        if window <= 0.0 {
            return 0.0; // already at its release point; nothing to be gained
        }
        air_defence::p_kill_in_window(&ad.stats, range_m, window)
    }
}
