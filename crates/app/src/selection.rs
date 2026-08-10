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

/// Where a selected asset is, or `None` if the index no longer resolves.
///
/// One place that knows how to look an asset up, so picking, the centroid, moving and the
/// panel's readout cannot disagree about where something is.
pub fn position(sim: &Sim, sel: Selected) -> Option<Vec2> {
    match sel {
        Selected::Unit(i) => sim.units().get(i).map(|u| u.pos),
        Selected::Air(i) => sim.air().get(i).map(|a| a.pos),
        Selected::AirDefence(i) => sim.air_defence().get(i).map(|d| d.pos),
        Selected::C2(i) => sim.c2().get(i).map(|c| c.pos),
    }
}

/// The nearest live asset to `pos` within `max_dist_m`, across every list.
pub fn nearest_asset(sim: &Sim, pos: Vec2, max_dist_m: f32) -> Option<Selected> {
    [
        sim.nearest_unit(pos, max_dist_m).map(Selected::Unit),
        sim.nearest_air(pos, max_dist_m).map(Selected::Air),
        sim.nearest_air_defence(pos, max_dist_m)
            .map(Selected::AirDefence),
        sim.nearest_c2(pos, max_dist_m).map(Selected::C2),
    ]
    .into_iter()
    .flatten()
    .filter_map(|sel| position(sim, sel).map(|p| (sel, p.distance(pos))))
    // Ties break on list order, so clicking between two coincident markers always picks
    // the same one - a battery and its command post can sit very close together.
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(sel, _)| sel)
}

/// Every live asset on the map, for select-all.
pub fn all_live_assets(sim: &Sim) -> Vec<Selected> {
    live_assets(sim, |_| true)
}

/// Every live asset inside the rectangle spanned by two world corners.
pub fn assets_in_box(sim: &Sim, a: Vec2, b: Vec2) -> Vec<Selected> {
    let (lo, hi) = (a.min(b), a.max(b));
    live_assets(sim, |p| {
        p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
    })
}

/// Every live asset whose position passes `keep`, in list order.
///
/// Select-all and box-select differ only in that test, so they share the traversal - which
/// is what stops a new asset class being added to one and forgotten in the other.
fn live_assets(sim: &Sim, keep: impl Fn(Vec2) -> bool) -> Vec<Selected> {
    let mut out = Vec::new();
    out.extend(
        sim.units()
            .iter()
            .enumerate()
            .filter(|(_, u)| u.alive() && keep(u.pos))
            .map(|(i, _)| Selected::Unit(i)),
    );
    out.extend(
        sim.air()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && keep(a.pos))
            .map(|(i, _)| Selected::Air(i)),
    );
    out.extend(
        sim.air_defence()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.alive() && keep(d.pos))
            .map(|(i, _)| Selected::AirDefence(i)),
    );
    out.extend(
        sim.c2()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.alive() && keep(c.pos))
            .map(|(i, _)| Selected::C2(i)),
    );
    out
}

/// The centroid of a selection, for formation-preserving moves.
pub fn selection_centroid(sim: &Sim, selected: &[Selected]) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    let mut n = 0.0f32;
    for sel in selected {
        if let Some(p) = position(sim, *sel) {
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
        let Some(offset) = position(sim, *sel).map(|p| p - centroid) else {
            continue;
        };
        let to = target + offset;
        match sel {
            Selected::Unit(i) => sim.set_unit_pos(*i, to),
            Selected::Air(i) => {
                let air = sim.air_mut(*i);
                air.pos = to;
                air.set_plan(FlightPlan::default());
            }
            // Emplaced: re-siting is the whole decision, and the battery's organic radar
            // moves with it (`Sim::set_air_defence_pos`).
            Selected::AirDefence(i) => sim.set_air_defence_pos(*i, to),
            Selected::C2(i) => sim.set_c2_pos(*i, to),
        }
    }
}

/// Append a waypoint to every selected asset, offset to preserve formation. A ground unit
/// gains a route waypoint; a drone gains a flight-plan leg.
///
/// Emplaced assets are skipped rather than teleported: shift+right-click means "and then go
/// here", and a battery that answered it by jumping would be doing something else entirely.
/// A mixed selection therefore routes what can move and leaves the rest where it is.
pub fn append_waypoint(sim: &mut Sim, selected: &[Selected], target: Vec2) {
    let Some(centroid) = selection_centroid(sim, selected) else {
        return;
    };
    for sel in selected {
        let Some(offset) = position(sim, *sel).map(|p| p - centroid) else {
            continue;
        };
        let to = target + offset;
        match sel {
            Selected::Unit(i) => sim.push_waypoint(*i, to),
            Selected::Air(i) => {
                let air = sim.air_mut(*i);
                if air.plan_complete() {
                    air.set_plan(FlightPlan::route(vec![to]));
                } else {
                    air.plan.waypoints.push(to);
                }
            }
            Selected::AirDefence(_) | Selected::C2(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::scenario::{Libraries, Scenario};
    use std::path::Path;

    /// `ad_c2` is the fixture because it is the only scenario fielding all four asset
    /// kinds at once - which is exactly the case that used to be half-supported.
    fn fixture() -> Option<Sim> {
        let dir = Path::new("../../scenarios");
        let libs = Libraries::load_dir(dir).ok()?;
        let scn = Scenario::load(&dir.join("ad_c2.toml")).ok()?;
        Sim::new(&scn, &libs, scn.default_seed).ok()
    }

    #[test]
    fn every_asset_kind_can_be_picked_and_select_all_agrees() {
        let Some(sim) = fixture() else {
            return; // scenarios/ not present
        };
        // Clicking exactly on an asset must return that asset, not something else nearby.
        for (i, d) in sim.air_defence().iter().enumerate() {
            assert_eq!(
                nearest_asset(&sim, d.pos, PICK_RADIUS_M),
                Some(Selected::AirDefence(i)),
                "battery {} at {:?}",
                d.id,
                d.pos
            );
        }
        for (i, c) in sim.c2().iter().enumerate() {
            assert_eq!(
                nearest_asset(&sim, c.pos, PICK_RADIUS_M),
                Some(Selected::C2(i)),
                "post {}",
                c.id
            );
        }

        let all = all_live_assets(&sim);
        assert_eq!(
            all.len(),
            sim.units().len() + sim.air().len() + sim.air_defence().len() + sim.c2().len(),
            "select-all must reach every list"
        );
        // Box-select over the whole map must agree with select-all, or the two traversals
        // have drifted apart - the bug this refactor removed.
        let t = sim.terrain();
        let far = t.width().max(t.height()) as f32 * t.transform().cell_size_m();
        let mut boxed = assets_in_box(&sim, Vec2::new(-far, -far), Vec2::new(far, far));
        let mut all_sorted = all;
        boxed.sort_by_key(|s| format!("{s:?}"));
        all_sorted.sort_by_key(|s| format!("{s:?}"));
        assert_eq!(boxed, all_sorted);
    }

    #[test]
    fn a_removed_battery_stops_being_selectable_and_its_radar_goes_with_it() {
        let Some(mut sim) = fixture() else {
            return;
        };
        let pos = sim.air_defence()[0].pos;
        assert_eq!(
            nearest_asset(&sim, pos, PICK_RADIUS_M),
            Some(Selected::AirDefence(0))
        );
        sim.remove_air_defence(0);
        // Not merely deselected: gone from picking, and from select-all.
        assert_ne!(
            nearest_asset(&sim, pos, PICK_RADIUS_M),
            Some(Selected::AirDefence(0))
        );
        assert!(!all_live_assets(&sim).contains(&Selected::AirDefence(0)));
        // §12: the launchers and the organic radar die together.
        if let Some(s) = sim.air_defence()[0].sensor_idx {
            assert!(!sim.sensor_active(s), "a dead battery must stop emitting");
        }
    }

    /// A battery's organic radar is an ordinary entry in the sensor list, so moving the
    /// battery without it would leave a detached eye at the old site - visible only as
    /// coverage in the wrong place.
    #[test]
    fn moving_a_battery_takes_its_radar_along() {
        let Some(mut sim) = fixture() else {
            return;
        };
        let Some(radar) = sim.air_defence()[0].sensor_idx else {
            return; // this battery has no organic sensor
        };
        let to = sim.air_defence()[0].pos + Vec2::new(750.0, -400.0);
        move_selection(&mut sim, &[Selected::AirDefence(0)], to);
        assert_eq!(sim.air_defence()[0].pos, to);
        assert_eq!(sim.sensors()[radar].pos, to);
    }

    /// Emplaced assets have no route, so shift+right-click must leave them alone rather
    /// than teleporting them - and must still route the movers in a mixed selection.
    #[test]
    fn appending_a_waypoint_skips_emplaced_assets() {
        let Some(mut sim) = fixture() else {
            return;
        };
        if sim.units().is_empty() {
            return;
        }
        let before = sim.air_defence()[0].pos;
        let selection = [Selected::Unit(0), Selected::AirDefence(0)];
        append_waypoint(&mut sim, &selection, Vec2::new(9000.0, 9000.0));
        assert_eq!(sim.air_defence()[0].pos, before, "battery must not move");
        assert!(!sim.units()[0].route.is_empty(), "unit must have routed");
    }
}
