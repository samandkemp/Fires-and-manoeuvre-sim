//! Turning clicks into a selection, and commanding the selection once it exists.
//!
//! Selection spans both asset lists, so a box-select can pick up a mixed group of ground
//! units and drones and the same commands apply to all of it. Moves and waypoints are
//! **formation-preserving**: each asset keeps its offset from the group centroid rather
//! than every marker stacking on the click point.

use bevy::prelude::*;
use sim_core::air::FlightPlan;
use sim_core::sim::Sim;

use crate::state::Selected;

/// Click-pick radius, metres: how near a click must land to grab a marker.
pub const PICK_RADIUS_M: f32 = 400.0;
/// A left-drag shorter than this is a click, not a box-select.
pub const BOX_SELECT_MIN_M: f32 = 60.0;

/// The nearest live asset to `pos` within `max_dist_m`, ground or air.
pub fn nearest_asset(sim: &Sim, pos: Vec2, max_dist_m: f32) -> Option<Selected> {
    let unit = sim
        .nearest_unit(pos, max_dist_m)
        .map(|i| (Selected::Unit(i), sim.units()[i].pos.distance(pos)));
    let air = sim
        .nearest_air(pos, max_dist_m)
        .map(|i| (Selected::Air(i), sim.air()[i].pos.distance(pos)));
    match (unit, air) {
        (Some(u), Some(a)) => Some(if a.1 < u.1 { a.0 } else { u.0 }),
        (Some(u), None) => Some(u.0),
        (None, Some(a)) => Some(a.0),
        (None, None) => None,
    }
}

/// Every live asset on the map, for select-all.
pub fn all_live_assets(sim: &Sim) -> Vec<Selected> {
    let mut out: Vec<Selected> = sim
        .units()
        .iter()
        .enumerate()
        .filter(|(_, u)| u.alive())
        .map(|(i, _)| Selected::Unit(i))
        .collect();
    out.extend(
        sim.air()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive)
            .map(|(i, _)| Selected::Air(i)),
    );
    out
}

/// Every live asset inside the rectangle spanned by two world corners.
pub fn assets_in_box(sim: &Sim, a: Vec2, b: Vec2) -> Vec<Selected> {
    let (lo, hi) = (a.min(b), a.max(b));
    let inside = |p: Vec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
    let mut out: Vec<Selected> = sim
        .units()
        .iter()
        .enumerate()
        .filter(|(_, u)| u.alive() && inside(u.pos))
        .map(|(i, _)| Selected::Unit(i))
        .collect();
    out.extend(
        sim.air()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && inside(a.pos))
            .map(|(i, _)| Selected::Air(i)),
    );
    out
}

/// The centroid of a selection, for formation-preserving moves.
pub fn selection_centroid(sim: &Sim, selected: &[Selected]) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    let mut n = 0.0f32;
    for sel in selected {
        let p = match sel {
            Selected::Unit(i) => sim.units().get(*i).map(|u| u.pos),
            Selected::Air(i) => sim.air().get(*i).map(|a| a.pos),
        };
        if let Some(p) = p {
            sum += p;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// Move a whole selection to `target`, **preserving formation**: each asset keeps its
/// offset from the group centroid rather than every marker stacking on the click point.
pub fn move_selection(sim: &mut Sim, selected: &[Selected], target: Vec2) {
    let Some(centroid) = selection_centroid(sim, selected) else {
        return;
    };
    for sel in selected {
        match sel {
            Selected::Unit(i) => {
                let offset = sim.units()[*i].pos - centroid;
                sim.set_unit_pos(*i, target + offset);
            }
            Selected::Air(i) => {
                let offset = sim.air()[*i].pos - centroid;
                let air = sim.air_mut(*i);
                air.pos = target + offset;
                air.set_plan(FlightPlan::default());
            }
        }
    }
}

/// Append a waypoint to every selected asset, offset to preserve formation. A ground unit
/// gains a route waypoint; a drone gains a flight-plan leg.
pub fn append_waypoint(sim: &mut Sim, selected: &[Selected], target: Vec2) {
    let Some(centroid) = selection_centroid(sim, selected) else {
        return;
    };
    for sel in selected {
        match sel {
            Selected::Unit(i) => {
                let offset = sim.units()[*i].pos - centroid;
                sim.push_waypoint(*i, target + offset);
            }
            Selected::Air(i) => {
                let offset = sim.air()[*i].pos - centroid;
                let air = sim.air_mut(*i);
                if air.plan_complete() {
                    air.set_plan(FlightPlan::route(vec![target + offset]));
                } else {
                    air.plan.waypoints.push(target + offset);
                }
            }
        }
    }
}
