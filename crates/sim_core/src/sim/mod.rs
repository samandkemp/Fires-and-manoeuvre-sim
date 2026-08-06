//! The simulation loop. Spec: `docs/DESIGN.md` §3.3, §9.6.
//!
//! Fixed-dt ticks integrate the stochastic sensing process; every `epoch_s` a decision
//! epoch maintains tracks and resolves fires. Deterministic given `(scenario, seed)`.
//!
//! # Where things live
//!
//! | Module | What it holds |
//! |---|---|
//! | this file | the [`Sim`] struct, [`Sim::step_one`] (the tick), and the read accessors |
//! | [`state`] | what a placed asset *is* — [`UnitState`], [`SensorState`], [`JammerState`] |
//! | [`events`] | the append-only logs every metric is read back from |
//! | [`setup`] | building a sim and placing assets into it |
//! | [`commands`] | what the app's mouse can change between ticks |
//! | [`detection`] | the glimpse process, EW, and the track lifecycle |
//! | [`engagement`] | ground fires: target selection and round resolution |
//! | [`counter_air`] | the air phases — air detection, air defence, strike release |
//!
//! Those are all child modules of `sim`, which is what lets them reach [`Sim`]'s private
//! fields while the rest of the crate cannot. Splitting the file cost no encapsulation.

use crate::air::AirState;
use crate::air_defence::AirDefenceState;
use crate::c2::C2State;
use crate::scenario::AllocationChoice;
use crate::suppression::Suppression;
use crate::terrain::TerrainGrid;
use crate::SimRng;
use glam::Vec2;
use rand::Rng;

mod commands;
mod counter_air;
mod detection;
mod engagement;
mod events;
mod los_cache;
mod setup;
mod state;
mod tasking;

pub use events::{AirDefenceEvent, AirDetectionEvent, DetectionEvent, FireEvent, StrikeEvent};
pub use state::{JammerState, SensorState, Side, UnitState};

use detection::SensorView;
use state::GlimpseTarget;

/// The simulation: terrain + assets + clock + seeded RNG, advanced by [`Sim::step_one`].
pub struct Sim {
    terrain: TerrainGrid,
    dt_s: f32,
    epoch_s: f32,
    // Suppression dials (from the scenario `[sim]` block, docs/DESIGN.md §4.3).
    suppression_radius_m: f32,
    p_suppress: f32,
    recover_per_s: f32,
    suppressed_fire_factor: f32,
    // Track lifecycle dials (§10.1).
    track_hold_s: f32,
    track_maintain_p: f32,
    // Fire-allocation dials (§10.2).
    allocation: AllocationChoice,
    max_shooters_per_target: u32,
    max_batteries_per_air_target: u32,
    // Sensor-tasking dials and state (§10.3).
    sensor_tasking: bool,
    tasking: tasking::Tasking,
    time_s: f64,
    epochs_run: u64,
    sensors: Vec<SensorState>,
    units: Vec<UnitState>,
    jammers: Vec<JammerState>,
    air: Vec<AirState>,
    air_defence: Vec<AirDefenceState>,
    c2: Vec<C2State>,
    events: Vec<DetectionEvent>,
    fire_events: Vec<FireEvent>,
    air_events: Vec<AirDetectionEvent>,
    air_defence_events: Vec<AirDefenceEvent>,
    strike_events: Vec<StrikeEvent>,
    // Scratch buffers, reused across epochs so a long battle does not allocate per epoch.
    // They carry no state between epochs — each user clears before filling.
    near_misses: Vec<u32>,
    views: Vec<(usize, SensorView)>,
    /// Memoised line-of-sight for (sensor, target) pairs whose endpoints have not moved.
    /// Purely a speed-up: a hit is exactly the value a miss would have computed.
    los_cache: los_cache::LosCache,
    rng: SimRng,
}

impl Sim {
    /// Advance one tick of `dt_s` seconds.
    ///
    /// The phase order is the determinism contract (`docs/DESIGN.md` §9.6). The air
    /// phases are **appended and draw zero RNG values when there are no air or
    /// air-defence assets**, so a drone-free scenario reproduces the pre-air event log
    /// bit-for-bit (V52) — the same identity posture EW takes (V40).
    pub fn step_one(&mut self) {
        self.time_s += f64::from(self.dt_s);

        // 1. Ground movement: advance each live, unpinned unit along its route.
        self.advance_units();

        // 2. Air movement — pure, no RNG, before sensing so positions are current.
        let dt = self.dt_s;
        for a in &mut self.air {
            a.advance(dt);
        }
        self.sync_carried_sensors();

        // 3. Sensing vs ground units. Unchanged draws and draw order from Phase 2:
        // `sensor_view` returns exactly the sensor's own position and mount height
        // unless it is carried, so a sim with no air is bit-identical here.
        self.detect_units();

        // 4. Sensing vs air. Zero iterations — and so zero draws — with no air assets.
        self.detect_air();

        // 5. Suppression recovery: memoryless per-tick step-down (fixed unit order).
        self.recover_suppression();

        // 6, 7. Counter-air and strike — both no-ops without air assets.
        self.resolve_air_defence();
        self.resolve_strikes();

        // 8. Decision epoch, in dependency order: refresh what is known, decide where to
        // look next, then decide what to shoot. Track maintenance leads because fires are
        // gated on tracks; tasking follows it because it reasons about what was *not*
        // seen this epoch.
        let epochs_due = (self.time_s / f64::from(self.epoch_s)).floor() as u64;
        while self.epochs_run < epochs_due {
            self.epochs_run += 1;
            self.maintain_tracks();
            self.task_sensors();
            self.resolve_fires();
        }
    }

    /// Step until sim time reaches at least `t_s`.
    pub fn run_until(&mut self, t_s: f64) {
        while self.time_s < t_s {
            self.step_one();
        }
    }

    /// Move every live, unpinned unit along its route (§6.1). Pure — a Pinned unit does
    /// not advance, which is the Phase 4 → Phase 5 wiring (V38).
    fn advance_units(&mut self) {
        let dt = self.dt_s;
        for u in &mut self.units {
            if u.elements == 0
                || u.suppression == Suppression::Pinned
                || u.speed_m_s <= 0.0
                || u.route_idx >= u.route.len()
            {
                continue;
            }
            let (pos, route_idx) = advance_along(u.pos, &u.route, u.route_idx, u.speed_m_s * dt);
            u.pos = pos;
            u.route_idx = route_idx;
        }
    }

    /// One memoryless step down the suppression chain per suppressed unit (§4.3).
    /// One draw per non-Free unit per tick, in fixed index order.
    fn recover_suppression(&mut self) {
        let p_recover = 1.0 - (-self.recover_per_s * self.dt_s).exp();
        for u_idx in 0..self.units.len() {
            if self.units[u_idx].suppression != Suppression::Free
                && self.rng.random::<f32>() < p_recover
            {
                self.units[u_idx].suppression = self.units[u_idx].suppression.step_down();
            }
        }
    }

    /// The terrain the sim runs over.
    #[must_use]
    pub fn terrain(&self) -> &TerrainGrid {
        &self.terrain
    }

    /// Current sim time, seconds.
    #[must_use]
    pub fn time_s(&self) -> f64 {
        self.time_s
    }

    /// Tick length, seconds.
    #[must_use]
    pub fn dt_s(&self) -> f32 {
        self.dt_s
    }

    /// Decision-epoch length, seconds (`docs/DESIGN.md` §3.3). Exposed so a front-end can
    /// step by the unit the *decisions* happen on, not just by the integration tick.
    #[must_use]
    pub fn epoch_s(&self) -> f32 {
        self.epoch_s
    }

    /// Decision epochs resolved so far (`docs/DESIGN.md` §3.3). One per `epoch_s` of sim
    /// time crossed, so it is the count of fires resolutions the run has performed.
    #[must_use]
    pub fn epochs_run(&self) -> u64 {
        self.epochs_run
    }

    /// All placed sensors, in placement order.
    #[must_use]
    pub fn sensors(&self) -> &[SensorState] {
        &self.sensors
    }

    /// All placed units, in placement order.
    #[must_use]
    pub fn units(&self) -> &[UnitState] {
        &self.units
    }

    /// Placed air assets, in placement order.
    #[must_use]
    pub fn air(&self) -> &[AirState] {
        &self.air
    }

    /// Placed air-defence batteries, in placement order.
    #[must_use]
    pub fn air_defence(&self) -> &[AirDefenceState] {
        &self.air_defence
    }

    /// Placed C2 posts, in placement order (`docs/DESIGN.md` §11).
    #[must_use]
    pub fn c2(&self) -> &[C2State] {
        &self.c2
    }

    /// The append-only detection log.
    #[must_use]
    pub fn events(&self) -> &[DetectionEvent] {
        &self.events
    }

    /// The append-only fires log.
    #[must_use]
    pub fn fire_events(&self) -> &[FireEvent] {
        &self.fire_events
    }

    /// The append-only air-detection log.
    #[must_use]
    pub fn air_events(&self) -> &[AirDetectionEvent] {
        &self.air_events
    }

    /// The append-only air-defence engagement log.
    #[must_use]
    pub fn air_defence_events(&self) -> &[AirDefenceEvent] {
        &self.air_defence_events
    }

    /// The append-only strike-release log.
    #[must_use]
    pub fn strike_events(&self) -> &[StrikeEvent] {
        &self.strike_events
    }
}

/// Advance `budget` metres along a polyline `route` from `pos` starting toward waypoint
/// `idx`, consuming multiple segments if the budget spans them. Returns the new position
/// and the next-waypoint index.
fn advance_along(mut pos: Vec2, route: &[Vec2], mut idx: usize, mut budget: f32) -> (Vec2, usize) {
    while budget > 0.0 && idx < route.len() {
        let to = route[idx] - pos;
        let dist = to.length();
        if dist <= budget {
            pos = route[idx];
            budget -= dist;
            idx += 1;
        } else if dist > 0.0 {
            pos += to / dist * budget;
            budget = 0.0;
        } else {
            idx += 1; // degenerate zero-length hop
        }
    }
    (pos, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fires::{WeaponClass, WeaponType};
    use crate::scenario::{load_terrain_params, Libraries, Scenario};
    use crate::sensing::{Modality, SensorType, UnitType};
    use std::collections::BTreeMap;
    use std::path::Path;

    /// A ground-only fight: one Blue sensor, one Blue direct-fire shooter, one Red
    /// target. Self-contained because the only gate left here must not depend on the
    /// validation crate's fixtures.
    fn ground_battle() -> (Scenario, Libraries) {
        let scn = Scenario::from_toml_str(
            r#"
            name = "zero-draw"
            default_seed = 5
            [sim]
            dt_s = 1.0
            epoch_s = 10.0
            [terrain]
            cell_size_m = 10.0
            width_cells = 64
            height_cells = 16
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.sensors]]
            id = "obs"
            type = "s"
            pos = [50.0, 80.0]
            [[blue.units]]
            id = "gun"
            type = "shooter"
            pos = [60.0, 80.0]
            [[red.units]]
            id = "tgt"
            type = "u"
            pos = [700.0, 80.0]
        "#,
        )
        .unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
        let libs = Libraries {
            sensors: BTreeMap::from([(
                "s".to_owned(),
                SensorType {
                    modality: Modality::Optical,
                    mount_height_m: 2.0,
                    max_range_m: 4000.0,
                    lambda0_per_s: 0.2,
                    range_half_m: 1200.0,
                    range_exponent: 2.0,
                    for_width_deg: None,
                },
            )]),
            units: BTreeMap::from([
                (
                    "u".to_owned(),
                    UnitType {
                        height_m: 2.0,
                        signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                        ..Default::default()
                    },
                ),
                (
                    "shooter".to_owned(),
                    UnitType {
                        height_m: 2.5,
                        silhouette_width_m: 3.0,
                        signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                        weapon: Some("cannon".to_owned()),
                        ..Default::default()
                    },
                ),
            ]),
            weapons: BTreeMap::from([(
                "cannon".to_owned(),
                WeaponType {
                    class: WeaponClass::Direct,
                    rof_rounds_per_min: 12.0,
                    max_range_m: 3000.0,
                    dispersion_mrad: 0.4,
                    p_kill_given_hit: 0.8,
                    ..Default::default()
                },
            )]),
            ..Libraries::with_terrain(load_terrain_params(&dir.join("terrain_types.toml")).unwrap())
        };
        (scn, libs)
    }

    // V52 (identity half): the air phases must draw **zero** RNG values when there are no
    // air or air-defence assets. Driving them repeatedly between ticks of an otherwise
    // ordinary ground run must therefore leave the event log bit-identical — if any of
    // them ever draws unconditionally, the streams diverge and this fails.
    #[test]
    fn v52_air_off_is_a_zero_draw_identity() {
        let (scn, libs) = ground_battle();

        let mut plain = Sim::new(&scn, &libs, 5).unwrap();
        plain.run_until(300.0);

        let mut hammered = Sim::new(&scn, &libs, 5).unwrap();
        while hammered.time_s() < 300.0 {
            for _ in 0..3 {
                hammered.detect_air();
                hammered.resolve_air_defence();
                hammered.resolve_strikes();
            }
            hammered.step_one();
        }

        assert_eq!(
            plain.events(),
            hammered.events(),
            "empty air phases must consume no randomness"
        );
        assert_eq!(plain.fire_events(), hammered.fire_events());
        assert!(
            hammered.air_events().is_empty()
                && hammered.air_defence_events().is_empty()
                && hammered.strike_events().is_empty(),
            "a scenario with no air must log no air activity"
        );
    }
}
