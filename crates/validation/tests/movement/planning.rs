//! V72-V74 - movement decisions in the loop. `docs/DESIGN.md` §5, §10.5.
//!
//! Fires are allocated and sensors are tasked, but until now a route was drawn by hand:
//! `movement::least_risk_path` was called only from `experiments/` and this crate, so the
//! dynamic-programming strand sat *beside* the model rather than inside it.
//!
//! A unit with an `objective` plans its own route each decision epoch against the live risk
//! raster. Three properties hold it honest, and they pull against each other:
//!
//! * **V72** - a scenario with no objective is bit-identical to before, and draws no extra
//!   randomness. Structural, not dial-gated: no objective means no planner at all.
//! * **V73** - a unit *avoids* what watches it, and with `risk_weight = 0` stops avoiding
//!   and takes the short way. Without the second half the first proves only that the router
//!   produces some route.
//! * **V74** - it does not dither. A unit re-deciding every epoch can flip between two
//!   near-equal routes forever; the hysteresis that prevents it is the movement analogue of
//!   §13.4's target lock.

use glam::Vec2;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::collections::BTreeMap;
use validation::scenario_params;

const START: Vec2 = Vec2::new(600.0, 2000.0);
const GOAL: Vec2 = Vec2::new(5400.0, 2000.0);
/// Straddling the straight line between them, so the direct route runs through it.
const WATCHER: Vec2 = Vec2::new(3000.0, 2000.0);

fn libraries() -> Libraries {
    let mut libs = Libraries::with_terrain(scenario_params());
    libs.units.insert(
        "mover".to_owned(),
        UnitType {
            height_m: 2.5,
            silhouette_width_m: 3.0,
            element_count: 1,
            speed_m_s: 12.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.7)]),
            ..Default::default()
        },
    );
    libs.sensors.insert(
        "watcher".to_owned(),
        SensorType {
            modality: Modality::Optical,
            mount_height_m: 8.0,
            // Wide enough that going around costs real distance, so the trade is a trade.
            max_range_m: 1400.0,
            lambda0_per_s: 1.0,
            range_half_m: 900.0,
            range_exponent: 2.0,
            for_width_deg: None,
        },
    );
    libs
}

/// Flat and open, so mobility is uniform and the *only* reason to deviate is being seen.
fn crossing(extra: &str) -> Scenario {
    Scenario::from_toml_str(&format!(
        r#"
        name = "planning"
        default_seed = 12
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 600
        height_cells = 400
        [terrain.source.flat]
        elevation_m = 0.0
        {extra}
    "#
    ))
    .expect("fixture parses")
}

/// A Blue mover with an objective, and a Red watcher on the direct line.
fn planned(risk_weight: &str) -> Sim {
    let scn = crossing(&format!(
        r#"
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
        objective = [{}, {}]
        {risk_weight}
        [[red.sensors]]
        id = "watcher"
        type = "watcher"
        pos = [{}, {}]
    "#,
        START.x, START.y, GOAL.x, GOAL.y, WATCHER.x, WATCHER.y
    ));
    Sim::new(&scn, &libraries(), 12).expect("fixture builds")
}

/// How far the unit ever strayed from the straight line between start and goal.
fn max_lateral_deviation(sim: &mut Sim, until_s: f64) -> f32 {
    let mut worst = 0.0f32;
    while sim.time_s() < until_s {
        sim.step_one();
        worst = worst.max((sim.units()[0].pos.y - START.y).abs());
        if sim.units()[0].pos.distance(GOAL) < 100.0 {
            break;
        }
    }
    worst
}

// ---------------------------------------------------------------------------------------
// V72 - the identity
// ---------------------------------------------------------------------------------------

// A scenario with no objective must be untouched: same event log, same everything. This is
// structural rather than a dial being off - `replan_movement` returns before doing anything
// if no unit has an objective, so there is no planner and no raster.
#[test]
fn v72_a_scenario_without_objectives_is_bit_identical() {
    let scn = crossing(&format!(
        r#"
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
        route = [[{}, {}], [{}, {}]]
        [[red.sensors]]
        id = "watcher"
        type = "watcher"
        pos = [{}, {}]
    "#,
        START.x, START.y, START.x, START.y, GOAL.x, GOAL.y, WATCHER.x, WATCHER.y
    ));
    let libs = libraries();
    let run = || {
        let mut s = Sim::new(&scn, &libs, 12).expect("builds");
        s.run_until(300.0);
        (
            s.units()[0].pos,
            s.events().to_vec(),
            s.fire_events().to_vec(),
        )
    };
    assert_eq!(run(), run(), "a scripted scenario must reproduce exactly");

    // And the scripted unit went where it was told, in a straight line - no planner touched
    // it. Any deviation would mean planning ran on a unit that never asked for it.
    let mut s = Sim::new(&scn, &libs, 12).expect("builds");
    let strayed = max_lateral_deviation(&mut s, 300.0);
    assert!(
        strayed < 1.0,
        "a scripted route must be followed exactly; strayed {strayed:.1} m"
    );
}

// Declaring both is refused at load rather than resolved by a precedence rule (§7.6's
// family): neither "plan then ignore the plan" nor "follow the route then re-plan" is
// obviously what was meant.
#[test]
fn v72_route_and_objective_together_are_a_load_error() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "both"
        default_seed = 1
        [terrain]
        cell_size_m = 10.0
        width_cells = 100
        height_cells = 100
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "confused"
        type = "mover"
        pos = [100.0, 100.0]
        route = [[200.0, 200.0]]
        objective = [{}, {}]
    "#,
        GOAL.x, GOAL.y
    ));
    let Err(e) = scn else {
        panic!("a unit with both a route and an objective must not load");
    };
    let msg = e.to_string();
    assert!(
        msg.contains("confused") && msg.contains("objective"),
        "the error must name the unit and the problem, got: {msg}"
    );
}

// ---------------------------------------------------------------------------------------
// V73 - it avoids what watches it, and only because it is watched
// ---------------------------------------------------------------------------------------

// The property the phase exists for: a sensor on the direct line pushes the route around it.
// Sensing now bites on *manoeuvre*, which a scripted waypoint list cannot express.
#[test]
fn v73_a_planner_routes_around_what_is_watching() {
    let cautious = max_lateral_deviation(&mut planned("risk_weight = 400.0"), 900.0);
    assert!(
        cautious > 300.0,
        "a watched corridor should push the route off the straight line; strayed only \
         {cautious:.0} m"
    );
}

// The other half, and the one that makes the first mean something: with the exchange rate at
// zero the unit stops caring who is watching and takes the short way. Same map, same sensor,
// same planner - this is V25's "zero risk is the shortest path" arriving in the loop.
#[test]
fn v73_with_no_risk_weight_it_takes_the_short_way() {
    let reckless = max_lateral_deviation(&mut planned("risk_weight = 0.0"), 900.0);
    assert!(
        reckless < 150.0,
        "with risk_weight = 0 the route should be essentially straight; strayed \
         {reckless:.0} m"
    );
}

// ---------------------------------------------------------------------------------------
// V74 - it does not dither
// ---------------------------------------------------------------------------------------

// A unit re-deciding every epoch can flip between two near-equal routes indefinitely,
// making no progress while looking busy. With a watcher squarely on the line, going north
// and going south cost almost exactly the same - which is precisely the situation that
// makes a fresh solve wobble.
//
// Note what is NOT the property: distance to the objective is *not* monotone, and should
// not be. A detour increases straight-line distance before it decreases it; that is what a
// detour is. Asserting monotone progress would forbid routing around anything at all - an
// earlier cut of this test did exactly that and failed for the right behaviour.
//
// The property is that the **committed direction does not flip**. Once the unit has decided
// to pass north of the watcher it stays north, and it arrives.
#[test]
fn v74_a_planner_does_not_flip_between_equal_routes() {
    let mut sim = planned("risk_weight = 400.0");
    let mut committed: Option<f32> = None;
    let mut flips = 0;

    while sim.time_s() < 1200.0 {
        sim.run_until(sim.time_s() + 10.0);
        let u = &sim.units()[0];
        if u.pos.distance(GOAL) < 100.0 {
            assert_eq!(
                flips, 0,
                "the route reversed {flips} time(s) before arriving"
            );
            return;
        }
        // Which side of the start-goal line the unit is actually committed to, taken from
        // where it is rather than from the plan, so this measures behaviour not intent.
        let offset = u.pos.y - START.y;
        if offset.abs() < 50.0 {
            continue; // still on the line: nothing committed yet
        }
        let side = offset.signum();
        match committed {
            None => committed = Some(side),
            Some(c) if c != side => {
                flips += 1;
                committed = Some(side);
            }
            _ => {}
        }
    }
    panic!(
        "never arrived; {:.0} m short after {flips} direction change(s)",
        sim.units()[0].pos.distance(GOAL)
    );
}

// A unit whose objective is unreachable must not wander or panic - it keeps whatever it has
// and the simulation carries on. Walled off by impassable terrain is the case; here the
// objective is simply where the unit already stands, the degenerate end of the same path.
#[test]
fn v74_an_objective_already_reached_is_stable() {
    let scn = crossing(&format!(
        r#"
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
        objective = [{}, {}]
    "#,
        GOAL.x, GOAL.y, GOAL.x, GOAL.y
    ));
    let mut sim = Sim::new(&scn, &libraries(), 12).expect("builds");
    sim.run_until(200.0);
    assert!(
        sim.units()[0].pos.distance(GOAL) < 50.0,
        "a unit already at its objective should stay there"
    );
    assert_eq!(sim.units()[0].side, Side::Blue);
}

// ---------------------------------------------------------------------------------------
// Setting an objective at runtime, as the app's mouse does
// ---------------------------------------------------------------------------------------

// The scenario loader refuses a unit declaring both a route and an objective. The same
// exclusivity has to hold when they are set interactively, or the planner would silently
// overwrite a hand-drawn route at the next epoch and the route would look like it had not
// taken.
#[test]
fn v72_setting_an_objective_and_a_route_stay_exclusive() {
    let scn = crossing(&format!(
        r#"
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
    "#,
        START.x, START.y
    ));
    let mut sim = Sim::new(&scn, &libraries(), 12).expect("builds");

    sim.set_objective(0, Some(GOAL));
    assert_eq!(sim.units()[0].objective, Some(GOAL));

    // A route now arrives from the mouse: the objective must go, or the planner keeps
    // overwriting what was just drawn.
    sim.set_route(0, vec![Vec2::new(1000.0, 1000.0)]);
    assert_eq!(
        sim.units()[0].objective,
        None,
        "a hand-drawn route must cancel the objective"
    );
    assert_eq!(sim.units()[0].route.len(), 1);

    // And back the other way: taking an objective drops the stale waypoints, so the first
    // planned route is computed against the current raster rather than inheriting one.
    sim.set_objective(0, Some(GOAL));
    assert!(
        sim.units()[0].route.is_empty(),
        "taking an objective must clear the route the planner is about to own"
    );

    // Clearing the objective leaves the unit where it is rather than planning on.
    sim.set_objective(0, None);
    assert_eq!(sim.units()[0].objective, None);
}

// An objective set at runtime must actually reach the planner, not merely be stored: the
// unit plans a route at the next epoch and starts moving along it.
#[test]
fn v72_a_runtime_objective_is_planned_and_followed() {
    let scn = crossing(&format!(
        r#"
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
    "#,
        START.x, START.y
    ));
    let mut sim = Sim::new(&scn, &libraries(), 12).expect("builds");
    sim.run_until(30.0);
    let parked = sim.units()[0].pos;
    assert!(
        parked.distance(START) < 1.0,
        "a unit with neither route nor objective must not move"
    );

    sim.set_objective(0, Some(GOAL));
    sim.set_unit_risk_weight(0, Some(0.0));
    sim.run_until(120.0);

    assert!(
        !sim.units()[0].route.is_empty(),
        "the planner should have produced a route by the next epoch"
    );
    assert!(
        sim.units()[0].pos.distance(GOAL) < parked.distance(GOAL) - 100.0,
        "the unit should have made real progress toward the objective"
    );
}
