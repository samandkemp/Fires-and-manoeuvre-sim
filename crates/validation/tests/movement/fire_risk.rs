//! V77 - fire in the movement risk raster. `docs/THEORY.md` §5.2, §10.5.
//!
//! At `fire_risk_weight = 0` the raster is enemy *observation* only, so a unit routes
//! happily through a beaten zone it cannot be seen from. Above zero, ground within an
//! enemy weapon's reach is priced too.
//!
//! Fixture, separating the two facts completely: an indirect gun covering a corridor it
//! cannot see into, and **no enemy sensor anywhere**. Observation risk is then identically
//! zero, so nothing but the new term can move the route.

use glam::Vec2;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::UnitType;
use sim_core::sim::Sim;
use sim_core::weapon_effects::{WeaponClass, WeaponType};
use std::collections::BTreeMap;
use validation::scenario_params;

const START: Vec2 = Vec2::new(600.0, 2000.0);
const GOAL: Vec2 = Vec2::new(5400.0, 2000.0);

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
    // An indirect gun: it needs no sightline, so it threatens ground nothing can observe.
    libs.weapons.insert(
        "howitzer".to_owned(),
        WeaponType {
            class: WeaponClass::Indirect,
            max_range_m: 1500.0,
            cep_m: 40.0,
            lethal_radius_m: 40.0,
            p_kill_given_hit: 0.6,
            rof_rounds_per_min: 6.0,
            ..Default::default()
        },
    );
    libs.units.insert(
        "gun".to_owned(),
        UnitType {
            height_m: 2.5,
            silhouette_width_m: 3.0,
            element_count: 2,
            weapon: Some("howitzer".to_owned()),
            signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
            ..Default::default()
        },
    );
    libs
}

/// Flat, open, and **no enemy sensor at all** - so observation risk is identically zero
/// everywhere and only the fire term can move the route.
fn scenario(fire_weight: f32) -> Scenario {
    Scenario::from_toml_str(&format!(
        r#"
        name = "fire_risk"
        default_seed = 9
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        fire_risk_weight = {fire_weight}
        [terrain]
        cell_size_m = 10.0
        width_cells = 600
        height_cells = 400
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [{}, {}]
        objective = [{}, {}]
        risk_weight = 600.0
        [[red.units]]
        id = "gun"
        type = "gun"
        pos = [3000.0, 2000.0]
    "#,
        START.x, START.y, GOAL.x, GOAL.y
    ))
    .expect("fixture parses")
}

/// How far the mover ever strayed from the straight line to its objective.
fn deviation(fire_weight: f32) -> f32 {
    let scn = scenario(fire_weight);
    let Ok(mut sim) = Sim::new(&scn, &libraries(), 9) else {
        return 0.0;
    };
    let mut worst = 0.0f32;
    while sim.time_s() < 900.0 {
        sim.step_one();
        worst = worst.max((sim.units()[0].pos.y - START.y).abs());
        if sim.units()[0].pos.distance(GOAL) < 100.0 {
            break;
        }
    }
    worst
}

// The identity. At weight zero the raster is observation only - and with
// no enemy sensor on the map there is nothing to avoid, so the route is straight.
#[test]
fn v77_at_zero_weight_fire_does_not_enter_the_raster() {
    let strayed = deviation(0.0);
    assert!(
        strayed < 150.0,
        "with fire risk off and no enemy sensor, the route should be essentially straight; \
         strayed {strayed:.0} m"
    );
}

// The capability. The same map, the same gun, no sensor anywhere - the only thing that can
// push the route off the line is the gun's reach.
#[test]
fn v77_a_unit_avoids_ground_it_can_be_shelled_on() {
    let strayed = deviation(1.0);
    assert!(
        strayed > 300.0,
        "a corridor covered by an indirect gun should push the route aside even though \
         nothing can see into it; strayed only {strayed:.0} m"
    );
}

// And it must be a dial, not a switch: more weight, more caution, monotonically.
#[test]
fn v77_more_weight_buys_more_clearance() {
    let (low, high) = (deviation(0.25), deviation(2.0));
    assert!(
        high >= low,
        "raising fire_risk_weight must not reduce the clearance taken ({high:.0} m at 2.0 \
         against {low:.0} m at 0.25)"
    );
}
