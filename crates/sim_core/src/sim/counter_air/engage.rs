//! Deciding whether a battery may shoot, and resolving the shots that are due
//! (`docs/DESIGN.md` §9.4-§9.5, §11.2).
//!
//! The envelope and cueing gates live here; *who shoots at what*, once a side is
//! coordinated, lives in [`super::coordinate`].

use super::CoordinatedBattery;
use crate::air_defence;
use crate::sim::{AirDefenceEvent, Side, Sim};

impl Sim {
    /// One tick of air defence (§9.4-§9.5): drop engagements whose target died or left
    /// the envelope, resolve shots that are due, then open new ones on the nearest
    /// actionable targets while channels and magazine last. Fixed index order throughout,
    /// which is the determinism unit.
    pub(in crate::sim) fn resolve_air_defence(&mut self) {
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
        // Batteries under C2 defer their opening to a coordinated pass (§11) - one pass
        // **per side**. Indexed by `Side`, exactly as `doctrine` and `orders` are, because
        // a post coordinates its own side and nobody else's: pooling both sides into one
        // problem would score every battery under whichever side happened to be first in
        // the list, so a side with a post would inherit the enemy's fire plan.
        let mut coordinated: [Vec<CoordinatedBattery>; 2] = [Vec::new(), Vec::new()];
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
                    // A track must have arrived *and* aged through the cueing timeline -
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
                // Borrow the two fields separately - `self.air_defence` and `self.rng`
                // are disjoint, which the borrow checker accepts as long as we reach
                // them as fields rather than through a `&mut self` method.
                let ad = &mut self.air_defence[ad_idx];
                ad.drop_engagements(|t| engageable.get(t).copied().unwrap_or(false));
                ad.resolve_due(now, dt, &mut self.rng, &mut resolutions);
            }
            // A resolution is what a counter-battery track is taken back along (§12.4).
            // Recorded here rather than recovered later by scanning the event log.
            if !resolutions.is_empty() {
                self.air_defence[ad_idx].last_fired_s = Some(now);
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
            // by exactly the rule it always used - which is what keeps a scenario with no
            // C2 post bit-identical to the pre-C2 engine (V59).
            if self.coordinated(ad_idx) {
                let side = self.air_defence[ad_idx].side;
                coordinated[side as usize].push((ad_idx, engageable, ranges));
                continue;
            }

            // Commit new engagements, **doctrine tier first, then nearest**; ties break on
            // index so the order is deterministic. Being outside the net costs a battery
            // its coordination, not its orders - a lone gun still shoots what it was told
            // to shoot first, it just does not know what anyone else is shooting (§13.2).
            // With no doctrine every tier is 0 and this is exactly nearest-first.
            let side = self.air_defence[ad_idx].side;
            let mut candidates: Vec<(usize, usize, f32)> = engageable
                .iter()
                .enumerate()
                .filter(|(_, &e)| e)
                .map(|(i, _)| (self.air_tier(side, i), i, ranges[i]))
                .collect();
            candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.total_cmp(&b.2)).then(a.1.cmp(&b.1)));
            let candidates: Vec<(usize, f32)> =
                candidates.into_iter().map(|(_, i, r)| (i, r)).collect();
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

        // Fixed side order, as everywhere else a decision is taken for both sides at once
        // (`resolve_fires`), so the sequence of opened engagements is deterministic.
        for side in [Side::Blue, Side::Red] {
            self.open_coordinated(&coordinated[side as usize], side, now);
        }
    }

    /// Refresh every battery's C2 link (`docs/DESIGN.md` §11.2).
    ///
    /// Two things can take a battery out of the net without touching the battery:
    ///
    /// - **The post dies** - `covers_jammed` is false for a dead post, so the group
    ///   decoheres from the next tick (§12.2).
    /// - **The link is jammed** - an enemy jammer near the post pulls its effective radius
    ///   in, so the batteries on the flanks fall out first and the ones sitting on top of
    ///   it keep talking. SEAD hard-kills the post; EW soft-kills its reach.
    ///
    /// Coming *into* coverage is not instantaneous: the battery is in the net at
    /// `now + link_latency_s`, and dropping out clears that, so a battery jammed out and
    /// back in pays the joining cost again.
    ///
    /// Draws no randomness, and is exactly a no-op when there are no posts - which is what
    /// keeps a post-free scenario bit-identical to the pre-C2 engine (V59).
    pub(in crate::sim) fn update_c2_links(&mut self) {
        if self.c2.is_empty() {
            return;
        }
        let now = self.time_s;
        // The link quality at each post, computed once: it depends on the post's position
        // and the enemy's jammers, neither of which varies per battery.
        let quality: Vec<f32> = self
            .c2
            .iter()
            .map(|post| self.link_quality_at(post.pos, post.side))
            .collect();

        for i in 0..self.air_defence.len() {
            let ad_side = self.air_defence[i].side;
            let ad_pos = self.air_defence[i].pos;
            // The best link on offer: the shortest latency among the posts that reach it.
            // A battery inside two posts' radii joins on whichever is quicker, which is
            // the only sensible reading of "it has two headquarters".
            let best = self
                .c2
                .iter()
                .zip(&quality)
                .filter(|(post, &q)| post.side == ad_side && post.covers_jammed(ad_pos, q))
                .map(|(post, _)| post.stats.link_latency_s)
                .fold(f32::INFINITY, f32::min);

            self.air_defence[i].net_ready_at_s = if best.is_finite() {
                // Already in the net: keep the ready time it earned rather than restarting
                // the clock every tick, which would make any latency permanent.
                self.air_defence[i]
                    .net_ready_at_s
                    .or(Some(now + f64::from(best.max(0.0))))
            } else {
                None
            };
        }
    }

    /// Is this battery in a live friendly C2 net *and* through its joining latency
    /// (`docs/DESIGN.md` §11)?
    fn coordinated(&self, ad_idx: usize) -> bool {
        self.air_defence[ad_idx]
            .net_ready_at_s
            .is_some_and(|t| self.time_s >= t)
    }
}
