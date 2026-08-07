//! Finding an airframe (`docs/DESIGN.md` §9.1).
//!
//! The ground glimpse loop with two changes: the target's actor height comes from its
//! altitude, and it contributes no terrain concealment — it is not standing in the cell
//! below it. Canopy transmittance still applies, being a property of the sightline.

use crate::sim::{AirDetectionEvent, GlimpseTarget, Sim};

impl Sim {
    /// The glimpse process against airborne targets (§9.1). Same as the ground loop
    /// except the target's actor height comes from its altitude and it contributes no
    /// terrain concealment — it isn't standing in the cell below it. Canopy transmittance
    /// still applies, being a property of the sightline rather than the target.
    pub(in crate::sim) fn detect_air(&mut self) {
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
                    kind: crate::sim::los_cache::TargetKind::Air,
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
}
