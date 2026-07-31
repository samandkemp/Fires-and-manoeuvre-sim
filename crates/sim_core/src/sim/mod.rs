//! The simulation loop. Spec: `docs/DESIGN.md` §3.3.
//!
//! Fixed-dt ticks integrate the stochastic sensing process; every `epoch_s` a decision
//! epoch maintains tracks and resolves fires. Deterministic given `(scenario, seed)`.

use crate::air::{AirState, AirType, FlightPlan, TargetSpec};
use crate::air_defence::{AirDefenceState, AirDefenceType};
use crate::ew::{self, Jammer};
use crate::fires::{self, WeaponClass, WeaponType};
use crate::los;
use crate::scenario::{Libraries, Scenario, ScenarioError, TargetConfig};
use crate::sensing::{self, detection_rate_against, p_detect_tick, SensorType, UnitType};
use crate::suppression::Suppression;
use crate::terrain::TerrainGrid;
use crate::SimRng;
use glam::Vec2;
use rand::{Rng, SeedableRng};

mod counter_air;

/// Firing height of a ground shooter above its own ground, metres — the sightline and
/// slant-range origin for direct fire.
const SHOOTER_HEIGHT_M: f32 = 2.0;

/// Resolve an optional weapon-type id against the library, cloning the stat block.
/// Shared by unit weapons and air payloads so an unknown id fails the same way for both.
fn resolve_weapon(id: Option<&str>, libs: &Libraries) -> Result<Option<WeaponType>, ScenarioError> {
    match id {
        None => Ok(None),
        Some(w) => libs
            .weapons
            .get(w)
            .cloned()
            .map(Some)
            .ok_or_else(|| ScenarioError::Invalid(format!("unknown weapon type '{w}'"))),
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

/// Which force an asset belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// Blue force (the user's, conventionally).
    Blue,
    /// Red force.
    Red,
}

/// A placed jammer with its owning side.
#[derive(Clone, Copy, Debug)]
pub struct JammerState {
    /// Owning side (protects this side's units).
    pub side: Side,
    /// The jammer's effect parameters.
    pub jammer: Jammer,
}

/// A placed sensor with its resolved stat block.
#[derive(Clone, Debug)]
pub struct SensorState {
    /// Scenario id (unique per scenario, used in event display).
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres. For a carried sensor this is **kept in step with the
    /// airframe** each tick (see [`SensorState::carrier`]), so reading it is always safe.
    pub pos: Vec2,
    /// Facing, degrees (0° = east, CCW); only matters with a finite field of regard. A
    /// carried sensor faces where its airframe is pointing, synced each tick.
    pub facing_deg: f32,
    /// Resolved stat block.
    pub stats: SensorType,
    /// Index into [`Sim::air`] of the airframe carrying this sensor, if any. A carried
    /// sensor takes its position, height and facing from the airframe each tick — which
    /// is all a recce drone is: a mobile, elevated entry in the ordinary sensor list, so
    /// it flows through the ordinary detection loop with no special case.
    pub carrier: Option<usize>,
}

/// A placed unit with its resolved stat block.
#[derive(Clone, Debug)]
pub struct UnitState {
    /// Scenario id.
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres.
    pub pos: Vec2,
    /// Resolved stat block.
    pub stats: UnitType,
    /// Resolved weapon, if the unit carries one.
    pub weapon: Option<WeaponType>,
    /// Whether the *opposing* side currently holds a track on this unit.
    ///
    /// Derived from [`UnitState::last_seen_s`] and refreshed at each decision epoch, so
    /// every reader keeps working while the underlying model is now a decaying track
    /// rather than a permanent flag (`docs/DESIGN.md` §10.1).
    pub detected: bool,
    /// Sim time this unit was last observed by the opposing side, if ever.
    pub last_seen_s: Option<f64>,
    /// Sub-elements remaining (attrition removes them one at a time).
    pub elements: u32,
    /// Sub-elements the unit started with.
    pub initial_elements: u32,
    /// Suppression state (gates fire and movement).
    pub suppression: Suppression,
    /// Movement speed along the route, metres/second (`0` = static).
    pub speed_m_s: f32,
    /// Route waypoints (world metres); empty = no route.
    pub route: Vec<Vec2>,
    /// Index of the next waypoint to head for.
    pub route_idx: usize,
}

impl UnitState {
    /// Still in the fight (at least one element remaining).
    #[must_use]
    pub fn alive(&self) -> bool {
        self.elements > 0
    }

    /// Fractional strength in `[0, 1]` (remaining / initial), for display and Lanchester.
    #[must_use]
    pub fn strength(&self) -> f32 {
        if self.initial_elements == 0 {
            0.0
        } else {
            self.elements as f32 / self.initial_elements as f32
        }
    }
}

/// One detection: emitted into the append-only event log the moment it happens.
/// This log is the observation channel later phases (targeting, EW/POMDP) consume.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectionEvent {
    /// Sim time of the detection, seconds.
    pub time_s: f64,
    /// Index into [`Sim::sensors`] of the detecting sensor.
    pub sensor: usize,
    /// Index into [`Sim::units`] of the detected unit.
    pub unit: usize,
    /// Where the unit was when detected.
    pub unit_pos: Vec2,
}

/// One resolved fires effect: a shooter's rounds killed elements of a target this epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct FireEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index of the shooting unit.
    pub shooter: usize,
    /// Index of the target unit.
    pub target: usize,
    /// Sub-elements destroyed this epoch.
    pub casualties: u32,
    /// Whether this reduced the target to 0 elements (killed).
    pub killed: bool,
}

/// What the glimpse process needs to know about one candidate target, whether it is a
/// ground unit or an airframe. Bundled so [`Sim::glimpse`] can serve both passes with
/// one signature rather than nine positional arguments.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlimpseTarget {
    /// World position, metres.
    pub pos: Vec2,
    /// Height above the ground beneath it — the §1.2 actor height.
    pub height_m: f32,
    /// Signature in the *sensor's* modality.
    pub signature: f32,
    /// Terrain concealment in `[0, 1]`; always 0 for an airborne target, which is not
    /// standing in the cell below it (§9.1).
    pub concealment: f32,
    /// Owning side — selects whose jammers protect it.
    pub side: Side,
}

/// One air detection: a sensor picked up an enemy airframe (`docs/DESIGN.md` §9).
/// Separate from [`DetectionEvent`] because the two index different asset lists.
#[derive(Clone, Debug, PartialEq)]
pub struct AirDetectionEvent {
    /// Sim time of the detection, seconds.
    pub time_s: f64,
    /// Index into [`Sim::sensors`] of the detecting sensor.
    pub sensor: usize,
    /// Index into [`Sim::air`] of the detected airframe.
    pub air: usize,
    /// Where the airframe was when detected.
    pub air_pos: Vec2,
}

/// One resolved air-defence shot (`docs/DESIGN.md` §9.4).
#[derive(Clone, Debug, PartialEq)]
pub struct AirDefenceEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index into [`Sim::air_defence`] of the firing battery.
    pub battery: usize,
    /// Index into [`Sim::air`] of the engaged airframe.
    pub air: usize,
    /// Did this shot destroy it?
    pub killed: bool,
}

/// One munition released by a strike drone (`docs/DESIGN.md` §9.3).
#[derive(Clone, Debug, PartialEq)]
pub struct StrikeEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index into [`Sim::air`] of the releasing airframe.
    pub air: usize,
    /// The aim point the munition was released against.
    pub aim: Vec2,
    /// Where it actually burst (aim + the CEP-sampled miss).
    pub burst: Vec2,
    /// Ground sub-elements destroyed by the burst.
    pub casualties: u32,
}

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
    track_hold_s: f32,
    track_maintain_p: f32,
    time_s: f64,
    epochs_run: u64,
    sensors: Vec<SensorState>,
    units: Vec<UnitState>,
    jammers: Vec<JammerState>,
    air: Vec<AirState>,
    air_defence: Vec<AirDefenceState>,
    events: Vec<DetectionEvent>,
    fire_events: Vec<FireEvent>,
    air_events: Vec<AirDetectionEvent>,
    air_defence_events: Vec<AirDefenceEvent>,
    strike_events: Vec<StrikeEvent>,
    rng: SimRng,
}

impl Sim {
    /// Build a sim from a scenario: generate terrain, resolve every instance's type id
    /// against the stat-block libraries, seed the RNG.
    ///
    /// # Errors
    /// [`ScenarioError::Invalid`] for an unknown sensor/unit/weapon/air/air-defence
    /// type id.
    pub fn new(scenario: &Scenario, libs: &Libraries, seed: u64) -> Result<Self, ScenarioError> {
        let terrain = scenario.build_terrain(&libs.terrain_params, seed);
        let cfg = &scenario.sim;
        let mut sim = Sim {
            terrain,
            dt_s: cfg.dt_s,
            epoch_s: cfg.epoch_s,
            suppression_radius_m: cfg.suppression_radius_m,
            p_suppress: cfg.p_suppress,
            recover_per_s: cfg.recover_per_s,
            suppressed_fire_factor: cfg.suppressed_fire_factor,
            track_hold_s: cfg.track_hold_s,
            track_maintain_p: cfg.track_maintain_p,
            time_s: 0.0,
            epochs_run: 0,
            sensors: Vec::new(),
            units: Vec::new(),
            jammers: Vec::new(),
            air: Vec::new(),
            air_defence: Vec::new(),
            events: Vec::new(),
            fire_events: Vec::new(),
            air_events: Vec::new(),
            air_defence_events: Vec::new(),
            strike_events: Vec::new(),
            rng: SimRng::seed_from_u64(seed ^ 0x5EED_5EED_5EED_5EED),
        };
        sim.place_from_scenario(scenario, libs)?;
        Ok(sim)
    }

    /// Clear every placed asset and re-place the scenario, **keeping the terrain** and
    /// reseeding the RNG (`docs/DESIGN.md` §1.3).
    ///
    /// Batch Monte-Carlo needs this to be honest as well as fast. `Sim::new` derives both
    /// the terrain and the RNG stream from one seed, so looping it over seeds varies the
    /// map and the dice together — two sources of variance averaged at once, when the
    /// question is usually "what happens on *this* map, on average". Building terrain once
    /// and resetting per trial separates them, and skips regenerating a 1000x1000 raster
    /// every run.
    ///
    /// # Errors
    /// As [`Sim::new`], for an unknown type id.
    pub fn reset_to_scenario(
        &mut self,
        scenario: &Scenario,
        libs: &Libraries,
        seed: u64,
    ) -> Result<(), ScenarioError> {
        self.reset(seed);
        self.place_from_scenario(scenario, libs)
    }

    /// Resolve and place every asset a scenario declares, in a fixed side-then-list order
    /// (the determinism contract's placement half).
    fn place_from_scenario(
        &mut self,
        scenario: &Scenario,
        libs: &Libraries,
    ) -> Result<(), ScenarioError> {
        let sim = self;
        for (side, force) in [(Side::Blue, &scenario.blue), (Side::Red, &scenario.red)] {
            for j in &force.jammers {
                sim.add_jammer(side, Vec2::from(j.pos), j.power, j.radius_m);
            }
            for s in &force.sensors {
                let stats = libs.sensors.get(&s.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown sensor type '{}'", s.type_id))
                })?;
                sim.add_sensor(&s.id, side, Vec2::from(s.pos), s.facing_deg, stats.clone());
            }
            for u in &force.units {
                let stats = libs.units.get(&u.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown unit type '{}'", u.type_id))
                })?;
                let weapon = resolve_weapon(stats.weapon.as_deref(), libs)?;
                sim.add_unit(&u.id, side, Vec2::from(u.pos), stats.clone(), weapon);
                if !u.route.is_empty() {
                    let idx = sim.units.len() - 1;
                    sim.set_route(idx, u.route.iter().map(|&p| Vec2::from(p)).collect());
                }
            }
            for a in &force.air {
                let stats = libs.air.get(&a.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown air type '{}'", a.type_id))
                })?;
                let sensor = match &stats.sensor {
                    Some(sid) => Some(
                        libs.sensors
                            .get(sid)
                            .ok_or_else(|| {
                                ScenarioError::Invalid(format!("unknown sensor type '{sid}'"))
                            })?
                            .clone(),
                    ),
                    None => None,
                };
                let payload = resolve_weapon(stats.payload.as_deref(), libs)?;
                let idx = sim.add_air(
                    &a.id,
                    side,
                    Vec2::from(a.pos),
                    a.altitude_m,
                    a.altitude_ref,
                    a.heading_deg,
                    stats.clone(),
                    sensor,
                    payload,
                );
                if let Some(speed) = a.speed_m_s {
                    sim.air[idx].speed_m_s = speed;
                }
                sim.air[idx].set_plan(FlightPlan {
                    waypoints: a.waypoints.iter().map(|&p| Vec2::from(p)).collect(),
                    terminal: a.terminal,
                });
                sim.air[idx].target = a.target.as_ref().map(|t| match t {
                    TargetConfig::Unit(id) => TargetSpec::Unit(id.clone()),
                    TargetConfig::Point(p) => TargetSpec::Point(Vec2::from(*p)),
                });
            }
            for d in &force.air_defence {
                let stats = libs.air_defence.get(&d.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown air-defence type '{}'", d.type_id))
                })?;
                let sensor = match &stats.sensor {
                    Some(sid) => Some(
                        libs.sensors
                            .get(sid)
                            .ok_or_else(|| {
                                ScenarioError::Invalid(format!("unknown sensor type '{sid}'"))
                            })?
                            .clone(),
                    ),
                    None => None,
                };
                sim.add_air_defence(
                    &d.id,
                    side,
                    Vec2::from(d.pos),
                    stats.clone(),
                    d.self_cue,
                    sensor,
                );
            }
        }
        Ok(())
    }

    /// Place a sensor (scenario load or interactive placement).
    pub fn add_sensor(
        &mut self,
        id: &str,
        side: Side,
        pos: Vec2,
        facing_deg: f32,
        stats: SensorType,
    ) {
        self.sensors.push(SensorState {
            id: id.to_owned(),
            side,
            pos,
            facing_deg,
            stats,
            carrier: None,
        });
    }

    /// Place an air asset, returning its index in [`Sim::air`]. A carried sensor is
    /// registered in the ordinary sensor list, bound to this airframe — so a recce drone
    /// needs no special case anywhere in the detection loop.
    #[allow(clippy::too_many_arguments)]
    pub fn add_air(
        &mut self,
        id: &str,
        side: Side,
        pos: Vec2,
        altitude_m: f32,
        altitude_ref: crate::air::AltitudeRef,
        heading_deg: f32,
        stats: AirType,
        sensor: Option<SensorType>,
        payload: Option<WeaponType>,
    ) -> usize {
        let idx = self.air.len();
        if let Some(stats) = sensor.clone() {
            self.sensors.push(SensorState {
                id: format!("{id}-sensor"),
                side,
                pos,
                facing_deg: heading_deg,
                stats,
                carrier: Some(idx),
            });
        }
        self.air.push(AirState::new(
            id,
            side,
            pos,
            altitude_m,
            altitude_ref,
            heading_deg,
            stats,
            sensor,
            payload,
        ));
        idx
    }

    /// Place an air-defence battery, returning its index in [`Sim::air_defence`]. An
    /// organic sensor goes in the ordinary sensor list and is remembered by index, which
    /// is how the battery tells "my own radar saw it" from "it came over the net" (§9.5).
    pub fn add_air_defence(
        &mut self,
        id: &str,
        side: Side,
        pos: Vec2,
        stats: AirDefenceType,
        self_cue: bool,
        sensor: Option<SensorType>,
    ) -> usize {
        let sensor_idx = sensor.map(|s| {
            self.sensors.push(SensorState {
                id: format!("{id}-radar"),
                side,
                pos,
                facing_deg: 0.0,
                stats: s,
                carrier: None,
            });
            self.sensors.len() - 1
        });
        self.air_defence.push(AirDefenceState::new(
            id, side, pos, stats, self_cue, sensor_idx,
        ));
        self.air_defence.len() - 1
    }

    /// Assign a flight plan to a placed air asset.
    pub fn set_flight_plan(&mut self, air_idx: usize, plan: FlightPlan) {
        self.air[air_idx].set_plan(plan);
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

    /// Where a sensor effectively is: `(position, height above its own ground, facing)`.
    /// A ground sensor reports its own position and mount height; a carried one reports
    /// its airframe's. That is all a recce drone is.
    #[must_use]
    pub fn sensor_view(&self, sensor_idx: usize) -> (Vec2, f32, f32) {
        let s = &self.sensors[sensor_idx];
        match s.carrier {
            Some(a) if a < self.air.len() => {
                let air = &self.air[a];
                (air.pos, air.actor_height(&self.terrain), air.heading_deg)
            }
            _ => (s.pos, s.stats.mount_height_m, s.facing_deg),
        }
    }

    /// Copy each carried sensor's position and facing back from its airframe.
    ///
    /// The airframe is the source of truth and [`Sim::sensor_view`] reads through to it,
    /// but `SensorState.pos` is public, and leaving it frozen at the placement point made
    /// overlays and `duel_probe` plot a recce drone's sensor at its take-off point.
    /// Syncing once per tick makes the obvious thing correct. One pass over the sensor
    /// list, no randomness, and a no-op when nothing is carried.
    fn sync_carried_sensors(&mut self) {
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

    /// One (sensor, target) glimpse (§3.2): the rate, the EW modifier, and a single
    /// seeded draw. `true` if this tick detected the target.
    ///
    /// Both passes — ground and air — come through here so the rate model, jamming and
    /// draw accounting can't drift apart. They differ only in what they put in
    /// [`GlimpseTarget`] and what they record afterwards.
    ///
    /// One draw per eligible pair per tick, in fixed index order. That is the unit the
    /// determinism contract counts.
    fn glimpse(
        &mut self,
        sensor_idx: usize,
        view: (Vec2, f32, f32),
        target: GlimpseTarget,
    ) -> bool {
        let (s_pos, s_height, s_facing) = view;
        // EW modifier: the target's own side's jammers degrade the rate (exactly 1 when
        // there are no jammers, so EW-off reduces bit-for-bit to Phase 2 — V40).
        let lambda = detection_rate_against(
            &self.terrain,
            &self.sensors[sensor_idx].stats,
            s_pos,
            s_height,
            s_facing,
            target.pos,
            target.height_m,
            target.signature,
            target.concealment,
        ) * self.jamming_at(target.pos, target.side);
        if lambda <= 0.0 {
            return false;
        }
        self.rng.random::<f32>() < p_detect_tick(lambda, self.dt_s)
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

    /// Place a jammer for `side` (protects that side's units from enemy detection).
    pub fn add_jammer(&mut self, side: Side, pos: Vec2, power: f32, radius_m: f32) {
        self.jammers.push(JammerState {
            side,
            jammer: Jammer {
                pos,
                power,
                radius_m,
            },
        });
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

    /// Place a unit (scenario load or interactive placement).
    pub fn add_unit(
        &mut self,
        id: &str,
        side: Side,
        pos: Vec2,
        stats: UnitType,
        weapon: Option<WeaponType>,
    ) {
        let elements = stats.element_count.max(1);
        let speed_m_s = stats.speed_m_s;
        self.units.push(UnitState {
            id: id.to_owned(),
            side,
            pos,
            stats,
            weapon,
            detected: false,
            last_seen_s: None,
            elements,
            initial_elements: elements,
            suppression: Suppression::Free,
            speed_m_s,
            route: Vec::new(),
            route_idx: 0,
        });
    }

    /// Assign a movement route (world waypoints) to a placed unit.
    pub fn set_route(&mut self, unit_idx: usize, route: Vec<Vec2>) {
        self.units[unit_idx].route = route;
        self.units[unit_idx].route_idx = 0;
    }

    /// Append one waypoint to a unit's route (for interactive route drawing).
    pub fn push_waypoint(&mut self, unit_idx: usize, waypoint: Vec2) {
        self.units[unit_idx].route.push(waypoint);
    }

    /// Teleport a unit to `pos`, clearing any route it was following.
    pub fn set_unit_pos(&mut self, unit_idx: usize, pos: Vec2) {
        let u = &mut self.units[unit_idx];
        u.pos = pos;
        u.route.clear();
        u.route_idx = 0;
    }

    /// Index of the nearest live air asset to `pos` within `max_dist_m`, or `None`.
    /// Horizontal distance: this is for picking a marker off a 2-D map.
    #[must_use]
    pub fn nearest_air(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        self.air
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive)
            .map(|(i, a)| (i, a.pos.distance(pos)))
            .filter(|(_, d)| *d <= max_dist_m)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Mutable access to a placed air asset — the hook for interactive editing (the app
    /// sets altitude, heading, speed and flight plan from its panel). Prefer the typed
    /// helpers elsewhere on `Sim` for anything the simulation itself does.
    pub fn air_mut(&mut self, air_idx: usize) -> &mut AirState {
        &mut self.air[air_idx]
    }

    /// Remove a unit from the fight.
    ///
    /// A tombstone, not a `Vec::remove`: the detection, fire, strike and air-defence logs
    /// all hold indices into these lists, as does `SensorState.carrier`, so shifting the
    /// vectors would repoint the whole recorded history at the wrong assets. Setting
    /// `elements = 0` is the state a killed unit already reaches, so nothing downstream
    /// needs a new case.
    pub fn remove_unit(&mut self, unit_idx: usize) {
        self.units[unit_idx].elements = 0;
        self.units[unit_idx].route.clear();
    }

    /// Remove an air asset, tombstoning it exactly as a shot-down airframe (see
    /// [`Sim::remove_unit`] for why indices are never shifted). Its carried sensor stops
    /// sensing with it, via [`Sim::sensor_active`].
    pub fn remove_air(&mut self, air_idx: usize) {
        self.air[air_idx].alive = false;
    }

    /// Index of the nearest unit to `pos` within `max_dist_m`, or `None`.
    #[must_use]
    pub fn nearest_unit(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        self.units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.alive())
            .map(|(i, u)| (i, u.pos.distance(pos)))
            .filter(|(_, d)| *d <= max_dist_m)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Clear all placed assets, events, and the clock, and reseed the RNG — keeping the
    /// (expensively generated) terrain. Lets a batch Monte-Carlo reuse one map across
    /// many trials instead of regenerating terrain each time.
    pub fn reset(&mut self, seed: u64) {
        self.time_s = 0.0;
        self.epochs_run = 0;
        self.sensors.clear();
        self.units.clear();
        self.jammers.clear();
        self.air.clear();
        self.air_defence.clear();
        self.events.clear();
        self.fire_events.clear();
        self.air_events.clear();
        self.air_defence_events.clear();
        self.strike_events.clear();
        self.rng = SimRng::seed_from_u64(seed ^ 0x5EED_5EED_5EED_5EED);
    }

    /// Advance one tick of `dt_s` seconds: run the glimpse process for every live
    /// (sensor, opposing undetected live target) pair in fixed index order, resolve air
    /// defence and strikes, then resolve fires if an epoch boundary was crossed.
    ///
    /// Phase order and the determinism contract are `docs/DESIGN.md` §9.6: the air
    /// phases are **appended and draw zero RNG values when there are no air or
    /// air-defence assets**, so a drone-free scenario reproduces the pre-air event log
    /// bit-for-bit (V52) — the same identity posture EW takes (V40).
    pub fn step_one(&mut self) {
        self.time_s += f64::from(self.dt_s);

        // 1. Movement: advance each live, unpinned unit along its route (Phase 4 → 5).
        for idx in 0..self.units.len() {
            let u = &self.units[idx];
            if !u.alive()
                || u.suppression == Suppression::Pinned
                || u.speed_m_s <= 0.0
                || u.route_idx >= u.route.len()
            {
                continue;
            }
            let (pos, route_idx) =
                advance_along(u.pos, &u.route, u.route_idx, u.speed_m_s * self.dt_s);
            self.units[idx].pos = pos;
            self.units[idx].route_idx = route_idx;
        }

        // 2. Air movement — pure, no RNG, before sensing so positions are current.
        let dt = self.dt_s;
        for a in &mut self.air {
            a.advance(dt);
        }
        self.sync_carried_sensors();

        // 3. Sensing vs ground units. Unchanged draws and draw order from Phase 2:
        // `sensor_view` returns exactly the sensor's own position and mount height
        // unless it is carried, so a sim with no air is bit-identical here.
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
                    pos: unit.pos,
                    height_m: unit.stats.height_m,
                    signature: unit.stats.signature_in(sensor.stats.modality),
                    // A ground unit's concealment is the terrain it stands in (§3.2).
                    concealment: sensing::concealment_at(&self.terrain, unit.pos),
                    side: unit.side,
                };
                if self.glimpse(s_idx, view, target) {
                    self.units[u_idx].detected = true;
                    self.units[u_idx].last_seen_s = Some(self.time_s);
                    self.events.push(DetectionEvent {
                        time_s: self.time_s,
                        sensor: s_idx,
                        unit: u_idx,
                        unit_pos: self.units[u_idx].pos,
                    });
                }
            }
        }

        // 4. Sensing vs air. Zero iterations — and so zero draws — with no air assets.
        self.detect_air();

        // 5. Suppression recovery: memoryless per-tick step-down (fixed unit order).
        let p_recover = 1.0 - (-self.recover_per_s * self.dt_s).exp();
        for u_idx in 0..self.units.len() {
            if self.units[u_idx].suppression != Suppression::Free
                && self.rng.random::<f32>() < p_recover
            {
                self.units[u_idx].suppression = self.units[u_idx].suppression.step_down();
            }
        }

        // 6, 7. Counter-air and strike — both no-ops without air assets.
        self.resolve_air_defence();
        self.resolve_strikes();

        // 8. Decision epoch: maintain tracks, then resolve one epoch of fires per
        // boundary crossed. Track maintenance leads because fires are gated on tracks.
        let epochs_due = (self.time_s / f64::from(self.epoch_s)).floor() as u64;
        while self.epochs_run < epochs_due {
            self.epochs_run += 1;
            self.maintain_tracks();
            self.resolve_fires();
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
    fn maintain_tracks(&mut self) {
        let (now, hold) = (self.time_s, f64::from(self.track_hold_s));

        // Which sensors can still see what, right now.
        let views: Vec<(usize, Vec2, f32, f32)> = (0..self.sensors.len())
            .filter(|&i| self.sensor_active(i))
            .map(|i| {
                let (pos, height, facing) = self.sensor_view(i);
                (i, pos, height, facing)
            })
            .collect();

        for u_idx in 0..self.units.len() {
            let unit = &self.units[u_idx];
            if unit.last_seen_s.is_none() || !unit.alive() {
                continue;
            }
            // Modality is per sensor, but every sensor is Optical today; take the
            // signature from the first view so the rate reflects the real target.
            let target = GlimpseTarget {
                pos: unit.pos,
                height_m: unit.stats.height_m,
                signature: views.first().map_or(0.0, |&(i, ..)| {
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
                pos: air.pos,
                height_m: air.actor_height(&self.terrain),
                signature: views.first().map_or(0.0, |&(i, ..)| {
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
    }

    /// Is any enemy sensor still seeing a target well enough to *hold* a track on it?
    ///
    /// Deliberately **not** a bare geometry test. A track is held when a sensor would
    /// expect to re-glimpse the target this epoch:
    /// `P(>=1 glimpse in epoch_s) = 1 - exp(-lambda_eff * epoch_s) >= track_maintain_p`.
    ///
    /// The full effective rate matters here: jamming, concealment, range and canopy all
    /// feed `lambda_eff`, so degrading a sensor enough *breaks* an existing track instead
    /// of only preventing a new one. A plain "can it be seen" test would leave EW unable
    /// to break anything, which is the gap this closes. Still deterministic — the rate
    /// decides, nothing is drawn.
    fn holds_track(&self, views: &[(usize, Vec2, f32, f32)], target: GlimpseTarget) -> bool {
        views.iter().any(|&(i, s_pos, s_height, s_facing)| {
            let sensor = &self.sensors[i];
            if sensor.side == target.side {
                return false;
            }
            let lambda = sensing::detection_rate_against(
                &self.terrain,
                &sensor.stats,
                s_pos,
                s_height,
                s_facing,
                target.pos,
                target.height_m,
                target.signature,
                target.concealment,
            ) * self.jamming_at(target.pos, target.side);
            p_detect_tick(lambda, self.epoch_s) >= self.track_maintain_p
        })
    }

    /// One epoch of fires. Each live, unpinned, weapon-carrying unit engages a target:
    /// direct fire takes the nearest enemy in clear LOS and range (no detection needed);
    /// indirect fire takes the nearest *detected* enemy in range. Fire volume scales with
    /// the shooter's live elements; suppression scales effectiveness. Rounds that land
    /// near the target but don't kill are near-misses that raise its suppression. All
    /// draws are in fixed index order — the determinism unit.
    fn resolve_fires(&mut self) {
        let mut near_misses = vec![0u32; self.units.len()];

        for s_idx in 0..self.units.len() {
            if !self.units[s_idx].alive() {
                continue;
            }
            let effectiveness = self.units[s_idx]
                .suppression
                .fire_effectiveness(self.suppressed_fire_factor);
            if effectiveness <= 0.0 {
                continue; // Pinned: no fire
            }
            let Some(weapon) = self.units[s_idx].weapon.clone() else {
                continue;
            };
            let shooter_side = self.units[s_idx].side;
            let shooter_pos = self.units[s_idx].pos;
            let elements = self.units[s_idx].elements;

            let Some(t_idx) = self.pick_target(shooter_side, shooter_pos, &weapon) else {
                continue;
            };
            let target_pos = self.units[t_idx].pos;
            let cover = self.cover_at(target_pos);

            // Every live element fires the weapon's per-element round count.
            let per_element = (weapon.rof_rounds_per_min * self.epoch_s / 60.0).round() as u32;
            let rounds = per_element.saturating_mul(elements);

            let mut remaining = self.units[t_idx].elements;
            let mut casualties = 0u32;
            for _ in 0..rounds {
                if remaining == 0 {
                    break;
                }
                let (killed, near) = self.fire_one_round(
                    &weapon,
                    shooter_pos,
                    target_pos,
                    t_idx,
                    cover,
                    effectiveness,
                    remaining,
                );
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
    }

    /// Resolve one round against the target: returns `(elements_killed, near_miss)`.
    /// `remaining` is the target's live element count before this round (indirect fire
    /// rolls each remaining element).
    #[allow(clippy::too_many_arguments)]
    fn fire_one_round(
        &mut self,
        weapon: &WeaponType,
        shooter_pos: Vec2,
        target_pos: Vec2,
        t_idx: usize,
        cover: f32,
        effectiveness: f32,
        remaining: u32,
    ) -> (u32, u32) {
        match weapon.class {
            WeaponClass::Direct => {
                // The round flies the slant distance, so dispersion scales with that.
                let range = los::slant_range(
                    &self.terrain,
                    shooter_pos,
                    SHOOTER_HEIGHT_M,
                    target_pos,
                    self.units[t_idx].stats.height_m,
                );
                let p_hit = fires::direct_p_hit(
                    weapon.dispersion_mrad,
                    range,
                    self.units[t_idx].stats.silhouette_width_m,
                    self.units[t_idx].stats.height_m,
                );
                let p_kill = p_hit * weapon.p_kill_given_hit * (1.0 - cover) * effectiveness;
                if self.rng.random::<f32>() < p_kill {
                    (1, 0) // element destroyed
                } else {
                    (0, 1) // round passed close — a near-miss
                }
            }
            WeaponClass::Indirect => {
                let sigma = fires::sigma_from_cep(weapon.cep_m);
                let burst = fires::sample_burst(target_pos, sigma, &mut self.rng);
                let miss = burst.distance(target_pos);
                let dmg = fires::carleton_damage(miss, weapon.lethal_radius_m)
                    * (1.0 - cover)
                    * effectiveness;
                // Each remaining element independently survives or not.
                let mut killed = 0u32;
                for _ in 0..remaining {
                    if self.rng.random::<f32>() < dmg {
                        killed += 1;
                    }
                }
                let near = u32::from(miss < self.suppression_radius_m);
                (killed, near)
            }
        }
    }

    /// The target a shooter engages this epoch, or `None`. Direct fire: nearest live
    /// enemy in clear LOS and range. Indirect fire: nearest live *detected* enemy in
    /// range (LOS not required).
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
            match weapon.class {
                WeaponClass::Direct => {
                    if !los::visible(
                        &self.terrain,
                        shooter_pos,
                        SHOOTER_HEIGHT_M,
                        u.pos,
                        u.stats.height_m,
                    ) {
                        continue;
                    }
                }
                WeaponClass::Indirect => {
                    if !u.detected {
                        continue;
                    }
                }
            }
            if best.is_none_or(|(_, br)| r < br) {
                best = Some((i, r));
            }
        }
        best.map(|(i, _)| i)
    }

    fn cover_at(&self, pos: Vec2) -> f32 {
        match self.terrain.transform().world_to_cell(pos) {
            Some((ix, iy)) => self.terrain.cover()[[iy, ix]],
            None => 0.0,
        }
    }

    /// Step until sim time reaches at least `t_s`.
    pub fn run_until(&mut self, t_s: f64) {
        while self.time_s < t_s {
            self.step_one();
        }
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

    /// Decision epochs resolved so far (`docs/DESIGN.md` §3.3). One per `epoch_s` of sim
    /// time crossed, so it is the count of fires resolutions the run has performed.
    #[must_use]
    pub fn epochs_run(&self) -> u64 {
        self.epochs_run
    }

    /// Force a unit's suppression state (`docs/DESIGN.md` §4.3).
    ///
    /// The suppression chain is normally driven by near-miss volume, but pinning a unit
    /// directly is what lets a caller isolate the *effect* of a state from the process
    /// that produces it — which is how V31 measures the fire-effectiveness multiplier and
    /// V38 checks that a pinned unit halts. Also the hook a scenario script or the app
    /// would use to set up a situation.
    pub fn set_suppression(&mut self, unit_idx: usize, state: Suppression) {
        self.units[unit_idx].suppression = state;
    }

    /// The terrain the sim runs over.
    #[must_use]
    pub fn terrain(&self) -> &TerrainGrid {
        &self.terrain
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::load_terrain_params;
    use crate::sensing::{Modality, UnitType};
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
