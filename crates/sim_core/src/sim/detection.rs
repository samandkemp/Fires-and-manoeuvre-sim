//! Seeing and being seen: the glimpse process, the EW modifier, and the track lifecycle.
//! Spec: `docs/DESIGN.md` §3.2, §8.1, §10.1. Gates: V14–V18, V40, V55.
//!
//! Three stages, and they are worth keeping distinct:
//!
//! 1. **Acquisition** — stochastic. One seeded draw per (sensor, unseen target) pair per
//!    tick, at rate `λ`. This is where a track is born.
//! 2. **Maintenance** — deterministic, at the decision epoch. A track already held is
//!    refreshed if a sensor would expect to re-glimpse the target. Keeping eyes on
//!    something you have already found is not a coin flip.
//! 3. **Expiry** — a track not refreshed within `track_hold_s` is lost, and the target
//!    must be acquired from scratch.

use super::los_cache::{self, TargetKind};
use super::{GlimpseTarget, JammerState, Side, Sim};
use crate::ew;
use crate::los::{self, LosResult};
use crate::sensing::{self, p_detect_tick};
use glam::Vec2;
use rand::Rng;

/// Where a sensor effectively is, as [`Sim::sensor_view`] reports it:
/// `(position, height above its own ground, facing)`.
pub(super) type SensorView = (Vec2, f32, f32);

impl Sim {
    /// Where a sensor effectively is: `(position, height above its own ground, facing)`.
    /// A ground sensor reports its own position and mount height; a carried one reports
    /// its airframe's. That is all a recce drone is.
    #[must_use]
    pub fn sensor_view(&self, sensor_idx: usize) -> SensorView {
        let s = &self.sensors[sensor_idx];
        match s.carrier {
            Some(a) if a < self.air.len() => {
                let air = &self.air[a];
                (air.pos, air.actor_height(&self.terrain), air.heading_deg)
            }
            _ => (s.pos, s.stats.mount_height_m, s.facing_deg),
        }
    }

    /// Is this sensor currently able to sense at all? A carried sensor dies with its
    /// airframe — so a shot-down recce drone's sensor must be excluded from coverage and
    /// belief rasters too, not just from the detection loop.
    #[must_use]
    pub fn sensor_active(&self, sensor_idx: usize) -> bool {
        match self.sensors[sensor_idx].carrier {
            Some(a) => self.air.get(a).is_some_and(|air| air.alive),
            None => true,
        }
    }

    /// Copy each carried sensor's position and facing back from its airframe.
    ///
    /// The airframe is the source of truth and [`Sim::sensor_view`] reads through to it,
    /// but `SensorState.pos` is public, and leaving it frozen at the placement point made
    /// overlays and `duel_probe` plot a recce drone's sensor at its take-off point.
    /// Syncing once per tick makes the obvious thing correct. One pass over the sensor
    /// list, no randomness, and a no-op when nothing is carried.
    pub(super) fn sync_carried_sensors(&mut self) {
        for s_idx in 0..self.sensors.len() {
            let Some(carrier) = self.sensors[s_idx].carrier else {
                continue;
            };
            let Some(air) = self.air.get(carrier) else {
                continue;
            };
            let (pos, facing) = (air.pos, air.heading_deg);
            let sensor = &mut self.sensors[s_idx];
            sensor.pos = pos;
            sensor.facing_deg = facing;
        }
    }

    /// Placed jammers.
    #[must_use]
    pub fn jammers(&self) -> &[JammerState] {
        &self.jammers
    }

    /// Detection-degradation factor at `pos` for a unit on `side` — the product of that
    /// side's own jammers covering the position (1 if none: EW-off identity).
    #[must_use]
    pub fn jamming_at(&self, pos: Vec2, side: Side) -> f32 {
        if self.jammers.is_empty() {
            return 1.0; // EW-off fast path (and exact identity)
        }
        // Fold the side's own jammers directly (no allocation on the hot path).
        let mut factor = 1.0f32;
        for js in &self.jammers {
            if js.side == side {
                factor *= ew::jamming_factor(pos, std::slice::from_ref(&js.jammer));
            }
        }
        factor
    }

    /// The effective glimpse rate of one sensor against one target: the §3.2 rate times
    /// the §8.1 jamming factor. Zero when blocked, out of range, or outside the field of
    /// regard.
    ///
    /// The single place the two are combined, so acquisition (a draw against this rate)
    /// and maintenance (a threshold on it) can never disagree about what a sensor can
    /// currently see.
    ///
    /// Gates first, then a *cached* line-of-sight walk (see [`los_cache`]) — the gates
    /// cost ~0.1 µs and the walk ~77 µs, and the walk's answer cannot change while both
    /// endpoints hold still.
    fn effective_rate(
        &mut self,
        sensor_idx: usize,
        view: SensorView,
        target: GlimpseTarget,
    ) -> f32 {
        let (s_pos, s_height, s_facing) = view;
        let Some(r) = sensing::detection_gate(
            &self.terrain,
            &self.sensors[sensor_idx].stats,
            s_pos,
            s_height,
            s_facing,
            target.pos,
            target.height_m,
            target.signature,
        ) else {
            return 0.0;
        };
        let los = self.cached_los(sensor_idx, target, s_pos, s_height);
        let rate = sensing::rate_given_los(
            &self.sensors[sensor_idx].stats,
            r,
            &los,
            target.signature,
            target.concealment,
        );
        if rate <= 0.0 {
            return 0.0; // hard-blocked: no need to price the jamming
        }
        rate * self.jamming_at(target.pos, target.side)
    }

    /// The line of sight from a sensor's viewpoint to a target, reusing the previous
    /// answer when neither endpoint has moved a single float ulp.
    fn cached_los(
        &mut self,
        sensor_idx: usize,
        target: GlimpseTarget,
        s_pos: Vec2,
        s_height: f32,
    ) -> LosResult {
        let key = los_cache::Key {
            sensor: sensor_idx,
            kind: target.kind,
            target: target.idx,
        };
        let at = los_cache::Endpoints {
            a: s_pos,
            h_a: s_height,
            b: target.pos,
            h_b: target.height_m,
        };
        self.los_cache
            .fit(self.sensors.len(), self.units.len(), self.air.len());
        if let Some(hit) = self.los_cache.get(key, at) {
            return hit;
        }
        let los = los::line_of_sight(&self.terrain, at.a, at.h_a, at.b, at.h_b);
        self.los_cache.put(key, at, los);
        los
    }

    /// Line-of-sight cache hits and misses so far — a diagnostic for checking the memo is
    /// earning its keep on a given scenario, not part of the model.
    #[must_use]
    pub fn los_cache_stats(&self) -> (u64, u64) {
        self.los_cache.stats()
    }

    /// One (sensor, target) glimpse (§3.2): the effective rate and a single seeded draw.
    /// `true` if this tick detected the target.
    ///
    /// Both passes — ground and air — come through here so the rate model, jamming and
    /// draw accounting cannot drift apart. They differ only in what they put in
    /// [`GlimpseTarget`] and what they record afterwards.
    ///
    /// One draw per eligible pair per tick, in fixed index order. That is the unit the
    /// determinism contract counts.
    pub(super) fn glimpse(
        &mut self,
        sensor_idx: usize,
        view: SensorView,
        target: GlimpseTarget,
    ) -> bool {
        let lambda = self.effective_rate(sensor_idx, view, target);
        if lambda <= 0.0 {
            return false;
        }
        self.rng.random::<f32>() < p_detect_tick(lambda, self.dt_s)
    }

    /// The glimpse process against every enemy ground unit not currently tracked (§3.2).
    ///
    /// Skipping already-tracked units is what keeps this cheap; refreshing those is
    /// [`Sim::maintain_tracks`]'s job, at epoch cadence.
    pub(super) fn detect_units(&mut self) {
        for s_idx in 0..self.sensors.len() {
            if !self.sensor_active(s_idx) {
                continue;
            }
            let view = self.sensor_view(s_idx);
            for u_idx in 0..self.units.len() {
                let (sensor, unit) = (&self.sensors[s_idx], &self.units[u_idx]);
                if unit.side == sensor.side || unit.detected || !unit.alive() {
                    continue;
                }
                let target = GlimpseTarget {
                    kind: TargetKind::Ground,
                    idx: u_idx,
                    pos: unit.pos,
                    height_m: unit.stats.height_m,
                    signature: unit.stats.signature_in(sensor.stats.modality),
                    // A ground unit's concealment is the terrain it stands in (§3.2).
                    concealment: sensing::concealment_at(&self.terrain, unit.pos),
                    side: unit.side,
                };
                if self.glimpse(s_idx, view, target) {
                    let (time_s, pos) = (self.time_s, self.units[u_idx].pos);
                    let unit = &mut self.units[u_idx];
                    unit.detected = true;
                    unit.last_seen_s = Some(time_s);
                    self.events.push(super::DetectionEvent {
                        time_s,
                        sensor: s_idx,
                        unit: u_idx,
                        unit_pos: pos,
                    });
                }
            }
        }
    }

    /// Refresh and expire tracks (`docs/DESIGN.md` §10.1).
    ///
    /// Runs at the **decision epoch, not the tick**, for two reasons. Conceptually,
    /// holding a track is a decision-layer concern. Practically, the glimpse loop skips
    /// already-detected targets, so refreshing means looking again — measured at 4 sensors
    /// x 6 units that is ~2.3 ms per tick, up to 20x the whole tick; at epoch cadence it
    /// amortises to ~0.23 ms, and tracks decay over tens of seconds so a 10 s cadence is
    /// ample.
    ///
    /// Refresh is **deterministic and draws no randomness**: acquiring a target is a
    /// stochastic glimpse, but keeping eyes on something already found is not a coin
    /// flip. That also leaves the per-tick RNG stream untouched.
    pub(super) fn maintain_tracks(&mut self) {
        let (now, hold) = (self.time_s, f64::from(self.track_hold_s));

        // Which sensors can still see anything, right now. Taken from the scratch buffer
        // so an epoch does not allocate.
        let mut views = std::mem::take(&mut self.views);
        views.clear();
        views.extend(
            (0..self.sensors.len())
                .filter(|&i| self.sensor_active(i))
                .map(|i| (i, self.sensor_view(i))),
        );

        for u_idx in 0..self.units.len() {
            let unit = &self.units[u_idx];
            if unit.last_seen_s.is_none() || !unit.alive() {
                continue;
            }
            // Modality is per sensor, but every sensor is Optical today; take the
            // signature from the first view so the rate reflects the real target.
            let target = GlimpseTarget {
                kind: TargetKind::Ground,
                idx: u_idx,
                pos: unit.pos,
                height_m: unit.stats.height_m,
                signature: views.first().map_or(0.0, |&(i, _)| {
                    unit.stats.signature_in(self.sensors[i].stats.modality)
                }),
                concealment: sensing::concealment_at(&self.terrain, unit.pos),
                side: unit.side,
            };
            if self.holds_track(&views, target) {
                self.units[u_idx].last_seen_s = Some(now);
            }
            // Expire: a track not re-observed within the hold time is lost, and the
            // target must be reacquired from scratch.
            let fresh = self.units[u_idx]
                .last_seen_s
                .is_some_and(|t| now - t < hold);
            self.units[u_idx].detected = fresh;
            if !fresh {
                self.units[u_idx].last_seen_s = None;
            }
        }

        for a_idx in 0..self.air.len() {
            let air = &self.air[a_idx];
            if air.last_seen_s.is_none() || !air.alive {
                continue;
            }
            let target = GlimpseTarget {
                kind: TargetKind::Air,
                idx: a_idx,
                pos: air.pos,
                height_m: air.actor_height(&self.terrain),
                signature: views.first().map_or(0.0, |&(i, _)| {
                    air.stats.signature_in(self.sensors[i].stats.modality)
                }),
                concealment: 0.0, // airborne: not standing in the cell below it (§9.1)
                side: air.side,
            };
            if self.holds_track(&views, target) {
                self.air[a_idx].last_seen_s = Some(now);
            }
            let fresh = self.air[a_idx].last_seen_s.is_some_and(|t| now - t < hold);
            self.air[a_idx].detected = fresh;
            if !fresh {
                // A lapsed track is *gone*: clear the cueing record too, so reacquisition
                // restarts the §9.5 timeline instead of a battery firing instantly off a
                // stale cue.
                let air = &mut self.air[a_idx];
                air.last_seen_s = None;
                air.detected_at_s = None;
                air.detected_by = None;
                air.seen_by.clear();
            }
        }

        self.views = views; // hand the buffer back for the next epoch
    }

    /// Is any enemy sensor still seeing a target well enough to *hold* a track on it?
    ///
    /// Deliberately **not** a bare geometry test. A track is held when a sensor would
    /// expect to re-glimpse the target this epoch:
    /// `P(>=1 glimpse in epoch_s) = 1 - exp(-lambda_eff * epoch_s) >= track_maintain_p`.
    ///
    /// The full effective rate matters here: jamming, concealment, range and canopy all
    /// feed it, so degrading a sensor enough *breaks* an existing track instead of only
    /// preventing a new one. A plain "can it be seen" test would leave EW unable to break
    /// anything, which is the gap this closes. Still deterministic — the rate decides,
    /// nothing is drawn.
    fn holds_track(&mut self, views: &[(usize, SensorView)], target: GlimpseTarget) -> bool {
        // Not `.any()` over an iterator: the rate lookup needs `&mut self` for the LOS
        // cache, which a closure capturing `self` cannot hand out. A plain loop is also
        // clearer about the short-circuit.
        for &(i, view) in views {
            if self.sensors[i].side == target.side {
                continue;
            }
            if p_detect_tick(self.effective_rate(i, view, target), self.epoch_s)
                >= self.track_maintain_p
            {
                return true;
            }
        }
        false
    }
}
