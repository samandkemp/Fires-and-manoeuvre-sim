//! Building a sim and placing assets into it.
//!
//! Placement order is part of the determinism contract: sides in a fixed order, and
//! within a side each asset list in the order the scenario declares it. Two runs of the
//! same scenario therefore agree on every index, which is what makes the event logs
//! comparable across runs.

use super::{JammerState, SensorState, Side, Sim, UnitState};
use crate::air::{AirState, AirType, FlightPlan, TargetSpec};
use crate::air_defence::{AirDefenceState, AirDefenceType};
use crate::c2::{C2State, C2Type};
use crate::doctrine::{Doctrine, Vocabulary};
use crate::ew::Jammer;
use crate::fires::WeaponType;
use crate::scenario::{Libraries, Scenario, ScenarioError, TargetConfig};
use crate::sensing::{SensorType, UnitType};
use crate::suppression::Suppression;
use crate::SimRng;
use glam::Vec2;
use rand::SeedableRng;

/// Mixed into the scenario seed so the RNG stream does not start at a "round" state.
const SEED_SALT: u64 = 0x5EED_5EED_5EED_5EED;

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

/// Look up a sensor stat block by id, with a uniform error. Both drones and air-defence
/// batteries may carry one, and both should fail the same way when the id is wrong.
fn resolve_sensor(
    id: Option<&String>,
    libs: &Libraries,
) -> Result<Option<SensorType>, ScenarioError> {
    match id {
        None => Ok(None),
        Some(sid) => libs
            .sensors
            .get(sid)
            .cloned()
            .map(Some)
            .ok_or_else(|| ScenarioError::Invalid(format!("unknown sensor type '{sid}'"))),
    }
}

impl Sim {
    /// Build a sim from a scenario: generate terrain, resolve every instance's type id
    /// against the stat-block libraries, seed the RNG.
    ///
    /// # Errors
    /// [`ScenarioError::Invalid`] for an unknown sensor/unit/weapon/air/air-defence
    /// type id.
    pub fn new(scenario: &Scenario, libs: &Libraries, seed: u64) -> Result<Self, ScenarioError> {
        // Again here, not only in `Libraries::load_dir`: a caller may have built the
        // libraries in code, or patched them in memory (`experiments/sweep`), and a dial
        // that would make a model evaluate to NaN should fail the same way either way.
        libs.validate()?;
        let cfg = &scenario.sim;
        let mut sim = Sim {
            terrain: scenario.build_terrain(&libs.terrain_params, seed),
            dt_s: cfg.dt_s,
            epoch_s: cfg.epoch_s,
            suppression_radius_m: cfg.suppression_radius_m,
            p_suppress: cfg.p_suppress,
            recover_per_s: cfg.recover_per_s,
            suppressed_fire_factor: cfg.suppressed_fire_factor,
            track_hold_s: cfg.track_hold_s,
            track_maintain_p: cfg.track_maintain_p,
            allocation: cfg.allocation,
            max_shooters_per_target: cfg.max_shooters_per_target,
            max_batteries_per_air_target: cfg.max_batteries_per_air_target,
            fires_need_c2: cfg.fires_need_c2,
            doctrine: [Doctrine::default(), Doctrine::default()],
            orders: [Vec::new(), Vec::new()],
            sensor_tasking: cfg.sensor_tasking,
            tasking: super::tasking::Tasking::new(cfg.belief_cells.max(1)),
            time_s: 0.0,
            epochs_run: 0,
            sensors: Vec::new(),
            units: Vec::new(),
            jammers: Vec::new(),
            air: Vec::new(),
            air_defence: Vec::new(),
            c2: Vec::new(),
            events: Vec::new(),
            fire_events: Vec::new(),
            air_events: Vec::new(),
            air_defence_events: Vec::new(),
            strike_events: Vec::new(),
            near_misses: Vec::new(),
            views: Vec::new(),
            los_cache: super::los_cache::LosCache::default(),
            rng: SimRng::seed_from_u64(seed ^ SEED_SALT),
        };
        sim.place_from_scenario(scenario, libs)?;
        Ok(sim)
    }

    /// Clear every placed asset and re-place the scenario, **keeping the terrain** and
    /// reseeding the RNG (`docs/DESIGN.md` §1.3).
    ///
    /// Batch Monte-Carlo needs this to be honest as well as fast. [`Sim::new`] derives
    /// both the terrain and the RNG stream from one seed, so looping it over seeds varies
    /// the map and the dice together — two sources of variance averaged at once, when the
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

    /// Clear all placed assets, events, and the clock, and reseed the RNG — keeping the
    /// (expensively generated) terrain.
    pub fn reset(&mut self, seed: u64) {
        self.time_s = 0.0;
        self.epochs_run = 0;
        self.sensors.clear();
        self.units.clear();
        self.jammers.clear();
        self.air.clear();
        self.air_defence.clear();
        self.c2.clear();
        self.events.clear();
        self.fire_events.clear();
        self.air_events.clear();
        self.air_defence_events.clear();
        self.strike_events.clear();
        // The cache is indexed by asset position in lists that are about to be rebuilt,
        // so its contents are meaningless rather than stale.
        self.los_cache.clear();
        // Likewise the belief: a new trial knows nothing about where the enemy is.
        self.tasking.reset();
        self.rng = SimRng::seed_from_u64(seed ^ SEED_SALT);
    }

    /// Resolve and place every asset a scenario declares, in a fixed side-then-list order
    /// (the determinism contract's placement half).
    fn place_from_scenario(
        &mut self,
        scenario: &Scenario,
        libs: &Libraries,
    ) -> Result<(), ScenarioError> {
        for (side, force) in [(Side::Blue, &scenario.blue), (Side::Red, &scenario.red)] {
            for j in &force.jammers {
                self.add_jammer(side, Vec2::from(j.pos), j.power, j.radius_m);
            }
            for s in &force.sensors {
                let stats = libs.sensors.get(&s.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown sensor type '{}'", s.type_id))
                })?;
                self.add_sensor(&s.id, side, Vec2::from(s.pos), s.facing_deg, stats.clone());
            }
            for u in &force.units {
                let stats = libs.units.get(&u.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown unit type '{}'", u.type_id))
                })?;
                let weapon = resolve_weapon(stats.weapon.as_deref(), libs)?;
                self.add_unit(&u.id, side, Vec2::from(u.pos), stats.clone(), weapon);
                if !u.route.is_empty() {
                    let idx = self.units.len() - 1;
                    self.set_route(idx, u.route.iter().map(|&p| Vec2::from(p)).collect());
                }
            }
            for a in &force.air {
                let stats = libs.air.get(&a.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown air type '{}'", a.type_id))
                })?;
                let sensor = resolve_sensor(stats.sensor.as_ref(), libs)?;
                let payload = resolve_weapon(stats.payload.as_deref(), libs)?;
                let idx = self.add_air(
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
                    self.air[idx].speed_m_s = speed;
                }
                self.air[idx].set_plan(FlightPlan {
                    waypoints: a.waypoints.iter().map(|&p| Vec2::from(p)).collect(),
                    terminal: a.terminal,
                });
                self.air[idx].target = a.target.as_ref().map(|t| match t {
                    TargetConfig::Unit(id) => TargetSpec::Named(id.clone()),
                    TargetConfig::Point(p) => TargetSpec::Point(Vec2::from(*p)),
                });
            }
            for d in &force.air_defence {
                let stats = libs.air_defence.get(&d.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown air-defence type '{}'", d.type_id))
                })?;
                let sensor = resolve_sensor(stats.sensor.as_ref(), libs)?;
                self.add_air_defence(
                    &d.id,
                    side,
                    Vec2::from(d.pos),
                    stats.clone(),
                    d.self_cue,
                    sensor,
                );
            }
            for c in &force.c2 {
                let stats = libs.c2.get(&c.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown C2 type '{}'", c.type_id))
                })?;
                self.add_c2(&c.id, side, Vec2::from(c.pos), stats.clone());
            }
            self.doctrine[side as usize] = force.doctrine.clone();
            self.orders[side as usize] = force.orders.clone();
        }
        // After placement, because a priority entry may name an asset by id and nothing is
        // placed until now.
        self.check_doctrine()
    }

    /// Every priority entry and every order must name something on the field
    /// (`docs/DESIGN.md` §13.1).
    ///
    /// A tier that matches nothing is not an empty tier — it is a doctrine nobody is
    /// following, and it fails silently: the run succeeds and simply answers a different
    /// question. Same reasoning as the schema's `deny_unknown_fields`, and the error names
    /// what *would* have worked, because the usual cause is a typo or a role never declared.
    fn check_doctrine(&self) -> Result<(), ScenarioError> {
        if self.doctrine.iter().all(Doctrine::is_undirected)
            && self.orders.iter().all(Vec::is_empty)
        {
            return Ok(()); // the default fire plan names only "all", which always matches
        }
        let mut vocab = Vocabulary::default();
        for t in self.all_target_names() {
            vocab.insert(&t);
        }
        for side in [Side::Blue, Side::Red] {
            if let Some(bad) = vocab.first_unmatched(&self.doctrine[side as usize].priority) {
                return Err(ScenarioError::Invalid(format!(
                    "{side:?} doctrine names '{bad}', which is not an id, role or class on \
                     this map. Known: {}",
                    vocab.known()
                )));
            }
            for order in &self.orders[side as usize] {
                for (what, id) in [("shooter", &order.shooter), ("target", &order.target)] {
                    if !vocab.0.contains(id) {
                        return Err(ScenarioError::Invalid(format!(
                            "{side:?} order names {what} '{id}', which is not on this map"
                        )));
                    }
                }
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
            engaging: None,
        });
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

    /// Place an air asset, returning its index in `Sim::air`. A carried sensor is
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

    /// Place a C2 post, returning its index in `Sim::c2` (`docs/DESIGN.md` §11).
    pub fn add_c2(&mut self, id: &str, side: Side, pos: Vec2, stats: C2Type) -> usize {
        self.c2.push(C2State::new(id, side, pos, stats));
        self.c2.len() - 1
    }

    /// Place an air-defence battery, returning its index in `Sim::air_defence`. An
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
}
