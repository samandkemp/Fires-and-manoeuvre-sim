//! Commands from outside the simulation: what the app's mouse and a scenario script can
//! change between ticks.
//!
//! Kept apart from the tick itself so the boundary is obvious — the simulation never
//! calls anything here, and everything here is safe to call between steps. Removal
//! **tombstones** rather than shifting the asset lists, because the event logs hold
//! indices into them (V54).

use super::Sim;
use crate::air::{AirState, FlightPlan};
use crate::scenario::AllocationChoice;
use crate::suppression::Suppression;
use glam::Vec2;

impl Sim {
    /// Which allocation rule the sides are using (`docs/DESIGN.md` §10.2).
    #[must_use]
    pub fn allocation(&self) -> AllocationChoice {
        self.allocation
    }

    /// Switch allocation rule mid-run.
    ///
    /// Safe between ticks and genuinely useful: running the same battle under `optimal`
    /// and `independent` is how the value of coordinating is *seen* rather than argued.
    /// The decision layer holds no state across epochs, so there is nothing to migrate.
    pub fn set_allocation(&mut self, choice: AllocationChoice) {
        self.allocation = choice;
    }

    /// Are steerable sensors re-pointing themselves (`docs/DESIGN.md` §10.3)?
    #[must_use]
    pub fn sensor_tasking(&self) -> bool {
        self.sensor_tasking
    }

    /// Turn belief-driven sensor tasking on or off mid-run. Switching it off leaves each
    /// sensor pointed wherever it last chose.
    pub fn set_sensor_tasking(&mut self, on: bool) {
        self.sensor_tasking = on;
    }

    /// Most air-defence batteries that may be assigned to one airframe
    /// (`docs/DESIGN.md` §11.2).
    #[must_use]
    pub fn max_batteries_per_air_target(&self) -> u32 {
        self.max_batteries_per_air_target
    }

    /// Set the air-defence overkill cap, clamped to at least 1 as the ground one is.
    ///
    /// Worth having live: 10,000 paired trials on `ad_c2` found the default of 2 buys no
    /// extra kills over 1 and costs a quarter of a round (§11.2), so this is a dial whose
    /// measured value is "none, on this scenario" — and watching a raid under 1, 2 and 3 is
    /// how that stops being a table and starts being obvious.
    pub fn set_max_batteries_per_air_target(&mut self, cap: u32) {
        self.max_batteries_per_air_target = cap.max(1);
    }

    /// Must a ground shooter be under a live friendly C2 post to join its side's
    /// coordinated fire plan (`docs/DESIGN.md` §11.3)?
    #[must_use]
    pub fn fires_need_c2(&self) -> bool {
        self.fires_need_c2
    }

    /// Turn the ground fire-control net requirement on or off mid-run.
    ///
    /// Safe between ticks: the split into netted and loose shooters is recomputed from
    /// scratch each epoch, so there is no state to migrate. Flipping it live is the
    /// clearest way to see §11.4's counter-intuitive result — that a *split* side can
    /// fight better, because the overkill cap applies once per fire-control problem and a
    /// loose shooter puts to work a slot the coordinated side would have idled.
    pub fn set_fires_need_c2(&mut self, on: bool) {
        self.fires_need_c2 = on;
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

    /// Assign a flight plan to a placed air asset.
    pub fn set_flight_plan(&mut self, air_idx: usize, plan: FlightPlan) {
        self.air[air_idx].set_plan(plan);
    }

    /// Mutable access to a placed air asset — the hook for interactive editing (the app
    /// sets altitude, heading, speed and flight plan from its panel). Prefer the typed
    /// helpers elsewhere on `Sim` for anything the simulation itself does.
    pub fn air_mut(&mut self, air_idx: usize) -> &mut AirState {
        &mut self.air[air_idx]
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

    /// Destroy a C2 post (`docs/DESIGN.md` §11).
    ///
    /// Tombstoned like every other removal. The interesting part is what it does *not*
    /// do: no battery is lost, no magazine emptied, no envelope shrunk. What is lost is
    /// the coordination — from the next tick, batteries that were allocating as a group
    /// revert to each taking whatever is nearest, with the duplicated engagements and
    /// leakers that follow. This is the hook SEAD will pull on.
    pub fn remove_c2(&mut self, c2_idx: usize) {
        self.c2[c2_idx].elements = 0;
    }

    /// Destroy an air-defence battery (`docs/DESIGN.md` §12), tombstoned like the rest.
    ///
    /// Two things go at once, which is what makes SEAD worth doing: the launchers stop
    /// engaging, **and** the organic radar goes dark — [`Sim::sensor_active`] already knows
    /// a battery's radar dies with it, so coverage and belief drop it without a special
    /// case here.
    pub fn remove_air_defence(&mut self, ad_idx: usize) {
        self.air_defence[ad_idx].elements = 0;
    }

    /// Reposition a C2 post. Emplaced assets do not move themselves, but siting one *is*
    /// the decision a C2 post represents, so dragging it around is how its radius is
    /// explored.
    pub fn set_c2_pos(&mut self, c2_idx: usize, pos: Vec2) {
        self.c2[c2_idx].pos = pos;
    }

    /// Reposition an air-defence battery, moving its organic radar with it.
    ///
    /// The radar is an ordinary entry in the sensor list, so leaving it behind would give
    /// the battery a detached eye at its old site — a bug that would show up only as
    /// coverage in the wrong place.
    pub fn set_air_defence_pos(&mut self, ad_idx: usize, pos: Vec2) {
        self.air_defence[ad_idx].pos = pos;
        if let Some(s) = self.air_defence[ad_idx].sensor_idx {
            self.sensors[s].pos = pos;
        }
    }

    /// The scenario id of whatever a fire event hit.
    ///
    /// Ground fires can now land on three different lists (`docs/DESIGN.md` §12.4), and
    /// every reader that wants to *name* the target — the app's feed, an experiment's
    /// report — would otherwise repeat the same three-armed match.
    #[must_use]
    pub fn fire_target_id(&self, target: super::FireTarget) -> &str {
        match target {
            super::FireTarget::Unit(i) => &self.units[i].id,
            super::FireTarget::AirDefence(i) => &self.air_defence[i].id,
            super::FireTarget::C2(i) => &self.c2[i].id,
        }
    }

    /// Index of the nearest live air-defence battery to `pos` within `max_dist_m`.
    #[must_use]
    pub fn nearest_air_defence(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        nearest(
            self.air_defence
                .iter()
                .enumerate()
                .filter(|(_, d)| d.alive())
                .map(|(i, d)| (i, d.pos)),
            pos,
            max_dist_m,
        )
    }

    /// Index of the nearest live C2 post to `pos` within `max_dist_m`.
    #[must_use]
    pub fn nearest_c2(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        nearest(
            self.c2
                .iter()
                .enumerate()
                .filter(|(_, c)| c.alive())
                .map(|(i, c)| (i, c.pos)),
            pos,
            max_dist_m,
        )
    }

    /// Index of the nearest live unit to `pos` within `max_dist_m`, or `None`.
    #[must_use]
    pub fn nearest_unit(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        nearest(
            self.units
                .iter()
                .enumerate()
                .filter(|(_, u)| u.alive())
                .map(|(i, u)| (i, u.pos)),
            pos,
            max_dist_m,
        )
    }

    /// Index of the nearest live air asset to `pos` within `max_dist_m`, or `None`.
    /// Horizontal distance: this is for picking a marker off a 2-D map.
    #[must_use]
    pub fn nearest_air(&self, pos: Vec2, max_dist_m: f32) -> Option<usize> {
        nearest(
            self.air
                .iter()
                .enumerate()
                .filter(|(_, a)| a.alive)
                .map(|(i, a)| (i, a.pos)),
            pos,
            max_dist_m,
        )
    }
}

/// Nearest `(index, position)` to `pos` within `max_dist_m`. Ties break on the lower
/// index, so clicking between two coincident markers always picks the same one.
fn nearest(
    candidates: impl Iterator<Item = (usize, Vec2)>,
    pos: Vec2,
    max_dist_m: f32,
) -> Option<usize> {
    candidates
        .map(|(i, p)| (i, p.distance(pos)))
        .filter(|(_, d)| *d <= max_dist_m)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(i, _)| i)
}
