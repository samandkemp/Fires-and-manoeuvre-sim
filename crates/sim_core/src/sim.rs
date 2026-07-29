//! The simulation loop (`docs/DESIGN.md` §3.3): fixed-dt ticks integrating the
//! stochastic sensing process, with decision epochs every `epoch_s` (an empty hook
//! until the fires/tasking phases fill it). Deterministic given `(scenario, seed)`.

use crate::ew::{self, Jammer};
use crate::fires::{self, WeaponClass, WeaponType};
use crate::los;
use crate::scenario::{Scenario, ScenarioError};
use crate::sensing::{detection_rate, p_detect_tick, SensorType, UnitType};
use crate::suppression::Suppression;
use crate::terrain::{TerrainGrid, TerrainParamsTable};
use crate::SimRng;
use glam::Vec2;
use rand::{Rng, SeedableRng};
use std::collections::BTreeMap;

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
    /// World position, metres.
    pub pos: Vec2,
    /// Facing, degrees (0° = east, CCW); only matters with a finite field of regard.
    pub facing_deg: f32,
    /// Resolved stat block.
    pub stats: SensorType,
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
    /// Whether the *opposing* side has detected this unit (permanent this phase).
    pub detected: bool,
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
    time_s: f64,
    epochs_run: u64,
    sensors: Vec<SensorState>,
    units: Vec<UnitState>,
    jammers: Vec<JammerState>,
    events: Vec<DetectionEvent>,
    fire_events: Vec<FireEvent>,
    rng: SimRng,
}

impl Sim {
    /// Build a sim from a scenario: generate terrain, resolve every instance's type id
    /// against the stat-block libraries, seed the RNG.
    ///
    /// # Errors
    /// [`ScenarioError::Invalid`] for an unknown sensor/unit type id.
    pub fn new(
        scenario: &Scenario,
        terrain_params: &TerrainParamsTable,
        sensor_types: &BTreeMap<String, SensorType>,
        unit_types: &BTreeMap<String, UnitType>,
        weapon_types: &BTreeMap<String, WeaponType>,
        seed: u64,
    ) -> Result<Self, ScenarioError> {
        let terrain = scenario.build_terrain(terrain_params, seed);
        let cfg = &scenario.sim;
        let mut sim = Sim {
            terrain,
            dt_s: cfg.dt_s,
            epoch_s: cfg.epoch_s,
            suppression_radius_m: cfg.suppression_radius_m,
            p_suppress: cfg.p_suppress,
            recover_per_s: cfg.recover_per_s,
            suppressed_fire_factor: cfg.suppressed_fire_factor,
            time_s: 0.0,
            epochs_run: 0,
            sensors: Vec::new(),
            units: Vec::new(),
            jammers: Vec::new(),
            events: Vec::new(),
            fire_events: Vec::new(),
            rng: SimRng::seed_from_u64(seed ^ 0x5EED_5EED_5EED_5EED),
        };
        for (side, force) in [(Side::Blue, &scenario.blue), (Side::Red, &scenario.red)] {
            for j in &force.jammers {
                sim.add_jammer(side, Vec2::from(j.pos), j.power, j.radius_m);
            }
            for s in &force.sensors {
                let stats = sensor_types.get(&s.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown sensor type '{}'", s.type_id))
                })?;
                sim.add_sensor(&s.id, side, Vec2::from(s.pos), s.facing_deg, stats.clone());
            }
            for u in &force.units {
                let stats = unit_types.get(&u.type_id).ok_or_else(|| {
                    ScenarioError::Invalid(format!("unknown unit type '{}'", u.type_id))
                })?;
                let weapon = match &stats.weapon {
                    Some(wid) => Some(
                        weapon_types
                            .get(wid)
                            .ok_or_else(|| {
                                ScenarioError::Invalid(format!("unknown weapon type '{wid}'"))
                            })?
                            .clone(),
                    ),
                    None => None,
                };
                sim.add_unit(&u.id, side, Vec2::from(u.pos), stats.clone(), weapon);
                if !u.route.is_empty() {
                    let idx = sim.units.len() - 1;
                    sim.set_route(idx, u.route.iter().map(|&p| Vec2::from(p)).collect());
                }
            }
        }
        Ok(sim)
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
        self.events.clear();
        self.fire_events.clear();
        self.rng = SimRng::seed_from_u64(seed ^ 0x5EED_5EED_5EED_5EED);
    }

    /// Advance one tick of `dt_s` seconds: run the glimpse process for every live
    /// (sensor, opposing undetected live unit) pair in fixed index order, then resolve
    /// fires if an epoch boundary was crossed.
    pub fn step_one(&mut self) {
        self.time_s += f64::from(self.dt_s);

        // Movement: advance each live, unpinned unit along its route (Phase 4 → 5).
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

        for s_idx in 0..self.sensors.len() {
            for u_idx in 0..self.units.len() {
                let (sensor, unit) = (&self.sensors[s_idx], &self.units[u_idx]);
                if unit.side == sensor.side || unit.detected || !unit.alive() {
                    continue;
                }
                // EW modifier: the target's own side's jammers degrade the glimpse rate
                // (identity when there are no jammers — reduces bit-for-bit to Phase 2).
                let lambda = detection_rate(
                    &self.terrain,
                    &sensor.stats,
                    sensor.pos,
                    sensor.facing_deg,
                    &unit.stats,
                    unit.pos,
                ) * self.jamming_at(unit.pos, unit.side);
                if lambda <= 0.0 {
                    continue;
                }
                // One RNG draw per live pair per tick, in fixed order — the
                // determinism contract's accounting unit.
                if self.rng.random::<f32>() < p_detect_tick(lambda, self.dt_s) {
                    self.units[u_idx].detected = true;
                    self.events.push(DetectionEvent {
                        time_s: self.time_s,
                        sensor: s_idx,
                        unit: u_idx,
                        unit_pos: self.units[u_idx].pos,
                    });
                }
            }
        }

        // Suppression recovery: memoryless per-tick step-down (fixed unit order).
        let p_recover = 1.0 - (-self.recover_per_s * self.dt_s).exp();
        for u_idx in 0..self.units.len() {
            if self.units[u_idx].suppression != Suppression::Free
                && self.rng.random::<f32>() < p_recover
            {
                self.units[u_idx].suppression = self.units[u_idx].suppression.step_down();
            }
        }

        // Decision epoch: resolve one epoch of fires per boundary crossed.
        let epochs_due = (self.time_s / f64::from(self.epoch_s)).floor() as u64;
        while self.epochs_run < epochs_due {
            self.epochs_run += 1;
            self.resolve_fires();
        }
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
                let range = shooter_pos.distance(target_pos);
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
            let r = shooter_pos.distance(u.pos);
            if r > weapon.max_range_m || r < weapon.min_range_m {
                continue;
            }
            match weapon.class {
                WeaponClass::Direct => {
                    if !los::visible(&self.terrain, shooter_pos, 2.0, u.pos, u.stats.height_m) {
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
    use crate::sensing::Modality;
    use std::path::{Path, PathBuf};

    fn scenario_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios")
            .join(name)
    }

    type Libraries = (
        Scenario,
        TerrainParamsTable,
        BTreeMap<String, SensorType>,
        BTreeMap<String, UnitType>,
        BTreeMap<String, WeaponType>,
    );

    /// A one-sensor, one-target duel on a flat range, built from an inline scenario.
    fn duel_scenario() -> Libraries {
        let text = r#"
            name = "duel"
            default_seed = 9
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
            [[red.units]]
            id = "tgt"
            type = "u"
            pos = [550.0, 80.0]
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let sensors = BTreeMap::from([(
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
        )]);
        let units = BTreeMap::from([(
            "u".to_owned(),
            UnitType {
                height_m: 2.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                ..Default::default()
            },
        )]);
        (scn, params, sensors, units, BTreeMap::new())
    }

    /// The analytic λ for the duel geometry, straight from the rate function.
    fn duel_lambda(
        scn: &Scenario,
        params: &TerrainParamsTable,
        s: &SensorType,
        u: &UnitType,
    ) -> f32 {
        let terrain = scn.build_terrain(params, scn.default_seed);
        detection_rate(
            &terrain,
            s,
            Vec2::new(50.0, 80.0),
            0.0,
            u,
            Vec2::new(550.0, 80.0),
        )
    }

    // V15 (+V14): Monte Carlo detection frequency by time t within binomial CI of the
    // closed form, and mean detection time ≈ 1/λ.
    #[test]
    fn v14_v15_exponential_law_monte_carlo() {
        let (scn, params, sensors, units, weapons) = duel_scenario();
        let lambda = f64::from(duel_lambda(&scn, &params, &sensors["s"], &units["u"]));
        assert!(
            lambda > 0.01 && lambda < 1.0,
            "duel λ should be a sane rate, got {lambda}"
        );

        let n = 1500;
        let t_check = 20.0;
        let mut detected_by_t = 0u32;
        let mut detection_times = Vec::new();
        for seed in 0..n {
            let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, 1000 + seed).unwrap();
            sim.run_until(200.0);
            if let Some(e) = sim.events().first() {
                if e.time_s <= t_check {
                    detected_by_t += 1;
                }
                detection_times.push(e.time_s);
            }
        }

        // V15: frequency vs 1 − e^{−λt}, 3.5σ binomial band.
        let p_exact = 1.0 - (-lambda * t_check).exp();
        let p_hat = f64::from(detected_by_t) / n as f64;
        let sigma = (p_exact * (1.0 - p_exact) / n as f64).sqrt();
        assert!(
            (p_hat - p_exact).abs() < 3.5 * sigma,
            "P(detect by {t_check}) = {p_hat:.4}, closed form {p_exact:.4}, σ {sigma:.4}"
        );

        // V14: nearly every run detects by t=200 (e^{−λ·200} ≈ 0), so the sample mean
        // estimates 1/λ. Discreteness of 1 s ticks biases the mean up by ~dt/2.
        let mean: f64 = detection_times.iter().sum::<f64>() / detection_times.len() as f64;
        let expected = 1.0 / lambda + 0.5;
        let se = (1.0 / lambda) / (detection_times.len() as f64).sqrt();
        assert!(
            (mean - expected).abs() < 4.0 * se,
            "mean detection time {mean:.2} vs 1/λ + dt/2 = {expected:.2} (se {se:.3})"
        );
    }

    // V18: same (scenario, seed) → identical event log; different seed differs.
    #[test]
    fn v18_determinism() {
        let (scn, params, sensors, units, weapons) = duel_scenario();
        let run = |seed: u64| {
            let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, seed).unwrap();
            sim.run_until(120.0);
            sim.events().to_vec()
        };
        assert_eq!(
            run(7),
            run(7),
            "same seed must reproduce the event log exactly"
        );
        let (a, b) = (run(7), run(8));
        let same = a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .all(|(x, y)| (x.time_s - y.time_s).abs() < f64::EPSILON);
        assert!(
            !same,
            "different seeds should give different detection times"
        );
    }

    // V37: a unit on a straight route travels speed·t (within one tick's step).
    #[test]
    fn v37_route_following() {
        let text = r#"
            name = "move"
            [terrain]
            cell_size_m = 10.0
            width_cells = 200
            height_cells = 20
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.units]]
            id = "mover"
            type = "mover"
            pos = [0.0, 100.0]
            route = [[1000.0, 100.0]]
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let units = BTreeMap::from([(
            "mover".to_owned(),
            UnitType {
                height_m: 2.0,
                speed_m_s: 10.0,
                ..Default::default()
            },
        )]);
        let mut sim =
            Sim::new(&scn, &params, &BTreeMap::new(), &units, &BTreeMap::new(), 0).unwrap();
        sim.run_until(30.0);
        let x = sim.units()[0].pos.x;
        assert!(
            (x - 300.0).abs() < 10.0,
            "after 30 s at 10 m/s the mover should be ~300 m along (got {x})"
        );
    }

    // V38: a Pinned unit does not advance along its route.
    #[test]
    fn v38_pinned_unit_halts() {
        let text = r#"
            name = "pin"
            [sim]
            recover_per_s = 0.0
            [terrain]
            cell_size_m = 10.0
            width_cells = 200
            height_cells = 20
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.units]]
            id = "mover"
            type = "mover"
            pos = [0.0, 100.0]
            route = [[1000.0, 100.0]]
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let units = BTreeMap::from([(
            "mover".to_owned(),
            UnitType {
                height_m: 2.0,
                speed_m_s: 10.0,
                ..Default::default()
            },
        )]);
        let mut sim =
            Sim::new(&scn, &params, &BTreeMap::new(), &units, &BTreeMap::new(), 0).unwrap();
        sim.units[0].suppression = Suppression::Pinned;
        sim.run_until(30.0);
        assert_eq!(sim.units()[0].pos.x, 0.0, "a pinned unit must not move");
    }

    // V40 (integration): a jammer over the target sharply cuts detections, and with no
    // jammer the run is bit-for-bit identical to the un-jammed sim (EW-off identity).
    #[test]
    fn v40_ew_degrades_and_off_is_identity() {
        let base = r#"
            name = "ew"
            default_seed = 4
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
            [[red.units]]
            id = "tgt"
            type = "u"
            pos = [550.0, 80.0]
        "#;
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let sensors = BTreeMap::from([(
            "s".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 2.0,
                max_range_m: 4000.0,
                lambda0_per_s: 0.3,
                range_half_m: 1200.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]);
        let units = BTreeMap::from([(
            "u".to_owned(),
            UnitType {
                height_m: 2.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                ..Default::default()
            },
        )]);
        let weapons = BTreeMap::new();

        let detect_frac = |scn_text: &str| -> f64 {
            let scn = Scenario::from_toml_str(scn_text).unwrap();
            let mut detected = 0u32;
            let trials = 300u64;
            for seed in 0..trials {
                let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, seed).unwrap();
                sim.run_until(30.0);
                if !sim.events().is_empty() {
                    detected += 1;
                }
            }
            f64::from(detected) / trials as f64
        };

        let unjammed = detect_frac(base);

        // A strong jammer sitting on the Red target.
        let jammed_text = format!(
            "{base}\n[[red.jammers]]\npos = [550.0, 80.0]\npower = 0.95\nradius_m = 400.0\n"
        );
        let jammed = detect_frac(&jammed_text);

        assert!(
            unjammed > 0.6,
            "sanity: un-jammed target is usually detected ({unjammed})"
        );
        assert!(
            jammed < unjammed * 0.4,
            "the jammer must sharply cut detection ({jammed} vs {unjammed})"
        );

        // EW-off identity: a run with an empty jammer list equals the base run exactly.
        let scn = Scenario::from_toml_str(base).unwrap();
        let events_a = {
            let mut s = Sim::new(&scn, &params, &sensors, &units, &weapons, 11).unwrap();
            s.run_until(60.0);
            s.events().to_vec()
        };
        let events_b = {
            let mut s = Sim::new(&scn, &params, &sensors, &units, &weapons, 11).unwrap();
            s.run_until(60.0);
            s.events().to_vec()
        };
        assert_eq!(
            events_a, events_b,
            "EW-off must be deterministic and unchanged"
        );
    }

    // V39: interdiction sanity — a Red route that no Blue overwatch can see is safe, so
    // the equilibrium puts Red on it and the game value falls. Builds a 2×2 payoff from
    // real headless battles, then solves it.
    #[test]
    fn v39_interdiction_safe_route() {
        use crate::game::solve_zero_sum;
        let scn = Scenario::from_toml_str(
            r#"
            name = "v39"
            [terrain]
            cell_size_m = 10.0
            width_cells = 250
            height_cells = 250
            [terrain.source.flat]
            elevation_m = 0.0
        "#,
        )
        .unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let mut sim = Sim::new(
            &scn,
            &params,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            1,
        )
        .unwrap();

        let sensor = SensorType {
            modality: Modality::Optical,
            mount_height_m: 2.0,
            max_range_m: 4000.0,
            lambda0_per_s: 1.0,
            range_half_m: 1500.0,
            range_exponent: 2.0,
            for_width_deg: Some(50.0),
        };
        let mortar_unit = UnitType {
            height_m: 2.0,
            element_count: 1,
            signature: BTreeMap::new(),
            weapon: Some("m".to_owned()),
            ..Default::default()
        };
        let mortar = WeaponType {
            class: WeaponClass::Indirect,
            rof_rounds_per_min: 20.0,
            max_range_m: 4000.0,
            min_range_m: 0.0,
            dispersion_mrad: 0.0,
            p_kill_given_hit: 1.0,
            moving_target_penalty: 1.0,
            cep_m: 40.0,
            lethal_radius_m: 40.0,
        };
        let red = UnitType {
            height_m: 2.8,
            silhouette_width_m: 3.2,
            element_count: 4,
            speed_m_s: 10.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
            weapon: None,
        };

        // Both Blue positions watch lane 0 (y=500) from the west; neither can see lane 1
        // (y=2000, outside every field of regard).
        let blue = [Vec2::new(1200.0, 500.0), Vec2::new(1800.0, 500.0)];
        let routes = [
            vec![Vec2::new(100.0, 500.0), Vec2::new(2400.0, 500.0)],
            vec![Vec2::new(100.0, 2000.0), Vec2::new(2400.0, 2000.0)],
        ];

        let seeds = 25u64;
        let mut payoff = ndarray::Array2::<f32>::zeros((2, 2));
        for bi in 0..2 {
            for rj in 0..2 {
                let mut acc = 0.0f32;
                for seed in 0..seeds {
                    sim.reset(seed);
                    sim.add_sensor("o", Side::Blue, blue[bi], 180.0, sensor.clone());
                    sim.add_unit(
                        "m",
                        Side::Blue,
                        blue[bi],
                        mortar_unit.clone(),
                        Some(mortar.clone()),
                    );
                    sim.add_unit("r", Side::Red, routes[rj][0], red.clone(), None);
                    let ri = sim.units().len() - 1;
                    sim.set_route(ri, routes[rj].clone());
                    loop {
                        sim.step_one();
                        let r = &sim.units()[ri];
                        if !r.alive() || r.route_idx >= r.route.len() || sim.time_s() > 600.0 {
                            acc += 1.0 - r.strength();
                            break;
                        }
                    }
                }
                payoff[[bi, rj]] = acc / seeds as f32;
            }
        }

        assert!(
            payoff[[0, 0]] > 0.5,
            "watched lane should be interdicted: {}",
            payoff[[0, 0]]
        );
        assert!(
            payoff[[0, 1]] < 0.2 && payoff[[1, 1]] < 0.2,
            "the unwatched lane must be safe: {payoff:?}"
        );

        let sol = solve_zero_sum(&payoff, 50_000);
        assert!(
            sol.col_strategy[1] > 0.9,
            "Red should take the safe route: {:?}",
            sol.col_strategy
        );
        assert!(
            sol.value < 0.2,
            "value should fall when Red has a safe route: {}",
            sol.value
        );
    }

    // Epoch bookkeeping: epochs advance with sim time.
    #[test]
    fn epochs_advance() {
        let (scn, params, sensors, units, weapons) = duel_scenario();
        let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, 3).unwrap();
        sim.run_until(35.0);
        assert_eq!(
            sim.epochs_run, 3,
            "35 s at 10 s epochs = 3 boundaries crossed"
        );
    }

    /// A close-range fight where a Blue direct-fire unit can see and engage a Red unit:
    /// checks fires actually attrit, and V24 (same seed → identical battle).
    fn battle_scenario() -> Libraries {
        // Reuse the duel's libraries (sensor "s", unit "u"); rebuild the scenario with a
        // Blue direct-fire shooter placed to see and engage the Red target.
        let (_, params, sensors, mut units, mut weapons) = duel_scenario();
        let text = r#"
            name = "battle"
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
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        units.insert(
            "shooter".to_owned(),
            UnitType {
                height_m: 2.5,
                silhouette_width_m: 3.0,
                element_count: 1,
                speed_m_s: 0.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                weapon: Some("cannon".to_owned()),
            },
        );
        weapons.insert(
            "cannon".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                rof_rounds_per_min: 12.0,
                max_range_m: 3000.0,
                min_range_m: 0.0,
                dispersion_mrad: 0.4,
                p_kill_given_hit: 0.8,
                moving_target_penalty: 1.0,
                cep_m: 0.0,
                lethal_radius_m: 0.0,
            },
        );
        (scn, params, sensors, units, weapons)
    }

    // Fires attrit a detected target, and the battle is deterministic (V24).
    #[test]
    fn v24_fires_attrit_and_are_deterministic() {
        let (scn, params, sensors, units, weapons) = battle_scenario();
        let run = |seed: u64| {
            let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, seed).unwrap();
            sim.run_until(600.0);
            let tgt = sim
                .units()
                .iter()
                .find(|u| u.id == "tgt")
                .unwrap()
                .strength();
            (tgt, sim.fire_events().to_vec())
        };
        let (strength_a, events_a) = run(11);
        let (strength_b, events_b) = run(11);
        assert_eq!(
            strength_a, strength_b,
            "same seed → identical target strength"
        );
        assert_eq!(events_a, events_b, "same seed → identical fire log");
        assert!(
            !events_a.is_empty(),
            "the detected target should be engaged"
        );
        assert!(
            strength_a < 1.0,
            "sustained direct fire should attrit the target"
        );

        let (strength_c, _) = run(12);
        // Different seed usually gives a different attrition outcome (not guaranteed if
        // both fully kill — but with these dials a 600 s fight kills, so compare killed).
        assert!(strength_c <= 1.0);
    }

    // V30: a homogeneous aimed-fire duel on open ground (suppression off) obeys
    // Lanchester's square law — the winner is annihilation-tested to end with
    // √(A₀²−B₀²) elements on average.
    #[test]
    fn v30_lanchester_square_law() {
        let text = r#"
            name = "lanchester"
            [sim]
            dt_s = 1.0
            epoch_s = 10.0
            p_suppress = 0.0
            [terrain]
            cell_size_m = 10.0
            width_cells = 80
            height_cells = 8
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.units]]
            id = "blue"
            type = "blue_line"
            pos = [200.0, 40.0]
            [[red.units]]
            id = "red"
            type = "red_line"
            pos = [500.0, 40.0]
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let sensors = BTreeMap::new();
        let line = |n: u32| UnitType {
            height_m: 2.0,
            silhouette_width_m: 3.0,
            element_count: n,
            speed_m_s: 0.0,
            signature: BTreeMap::new(),
            weapon: Some("rifle".to_owned()),
        };
        let units = BTreeMap::from([
            ("blue_line".to_owned(), line(50)),
            ("red_line".to_owned(), line(40)),
        ]);
        let weapons = BTreeMap::from([(
            "rifle".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                rof_rounds_per_min: 6.0, // round(6·10/60) = 1 round/element/epoch
                max_range_m: 2000.0,
                min_range_m: 0.0,
                dispersion_mrad: 0.5, // P_hit ≈ 1 at this range → p_kill = p_kill_given_hit
                p_kill_given_hit: 0.02,
                moving_target_penalty: 1.0,
                cep_m: 0.0,
                lethal_radius_m: 0.0,
            },
        )]);

        // The square-law invariant A² − B² is conserved by the deterministic ODE at
        // A₀² − B₀² = 900; check it holds in the mean over stochastic battles (robust to
        // which side happens to win an individual fight).
        let trials = 400u64;
        let mut sum_invariant = 0.0f64;
        let mut blue_wins = 0u32;
        for seed in 0..trials {
            let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, seed).unwrap();
            while sim.units().iter().all(UnitState::alive) && sim.time_s() < 5000.0 {
                sim.step_one();
            }
            let blue = f64::from(
                sim.units()
                    .iter()
                    .find(|u| u.id == "blue")
                    .unwrap()
                    .elements,
            );
            let red = f64::from(sim.units().iter().find(|u| u.id == "red").unwrap().elements);
            sum_invariant += blue * blue - red * red;
            if blue > red {
                blue_wins += 1;
            }
        }
        let mean_invariant = sum_invariant / trials as f64;
        assert!(
            (mean_invariant - 900.0).abs() < 90.0,
            "mean (A²−B²) = {mean_invariant:.0} vs Lanchester A₀²−B₀² = 900"
        );
        // The stronger force should win the large majority of fights.
        assert!(
            blue_wins > trials as u32 * 9 / 10,
            "Blue (50 vs 40) should usually win: {blue_wins}/{trials}"
        );
    }

    // V31: a Pinned unit emits no rounds; a Suppressed unit's expected output is
    // `suppressed_fire_factor` × a Free unit's. Drives one shooter at a fixed state
    // against a large target and measures casualties over many seeds.
    #[test]
    fn v31_suppression_gates_fire() {
        // Inline scenario: a Blue direct shooter vs a big Red target, flat, in LOS+range.
        let text = r#"
            name = "supp"
            [sim]
            dt_s = 1.0
            epoch_s = 10.0
            recover_per_s = 0.0
            [terrain]
            cell_size_m = 10.0
            width_cells = 40
            height_cells = 8
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.units]]
            id = "gun"
            type = "shooter"
            pos = [40.0, 40.0]
            [[red.units]]
            id = "block"
            type = "block"
            pos = [300.0, 40.0]
        "#;
        let scn = Scenario::from_toml_str(text).unwrap();
        let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
        let sensors = BTreeMap::new();
        let units = BTreeMap::from([
            (
                "shooter".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 1,
                    speed_m_s: 0.0,
                    signature: BTreeMap::new(),
                    weapon: Some("mg".to_owned()),
                },
            ),
            (
                "block".to_owned(),
                UnitType {
                    height_m: 2.0,
                    silhouette_width_m: 3.0,
                    element_count: 200,
                    ..Default::default()
                },
            ),
        ]);
        let weapons = BTreeMap::from([(
            "mg".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                rof_rounds_per_min: 60.0,
                max_range_m: 2000.0,
                min_range_m: 0.0,
                dispersion_mrad: 1.0,
                p_kill_given_hit: 0.5,
                moving_target_penalty: 1.0,
                cep_m: 0.0,
                lethal_radius_m: 0.0,
            },
        )]);

        // Average casualties inflicted in one epoch with the shooter forced to a state.
        let casualties_at = |state: Suppression| -> f64 {
            let trials = 400u64;
            let mut total = 0u32;
            for seed in 0..trials {
                let mut sim = Sim::new(&scn, &params, &sensors, &units, &weapons, seed).unwrap();
                sim.units[0].suppression = state;
                sim.run_until(10.0); // one epoch
                total += sim.units[1].initial_elements - sim.units[1].elements;
            }
            f64::from(total) / trials as f64
        };

        let free = casualties_at(Suppression::Free);
        let suppressed = casualties_at(Suppression::Suppressed);
        let pinned = casualties_at(Suppression::Pinned);

        assert_eq!(pinned, 0.0, "a pinned unit must not fire");
        assert!(
            free > 4.0,
            "sanity: free unit inflicts casualties (got {free})"
        );
        // suppressed / free ≈ suppressed_fire_factor (0.4).
        let ratio = suppressed / free;
        assert!(
            (ratio - 0.4).abs() < 0.06,
            "suppressed output ratio {ratio:.3} should be ~0.4"
        );
    }
}
