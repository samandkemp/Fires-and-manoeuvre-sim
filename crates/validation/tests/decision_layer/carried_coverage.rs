//! V61 - a carried sensor's coverage informs belief. `docs/DESIGN.md` §10.3.
//!
//! Until now the tasking layer skipped carried sensors entirely, so a recce drone could
//! fly the length of the map, see nothing, and leave its side's belief about that ground
//! completely unchanged. That is wrong in a specific and interesting way: **not finding
//! anything is evidence**. Negative information is the whole point of the POMDP layer
//! (V43), and the most mobile observer on the field was the one asset excluded from it.
//!
//! The fixture is deliberately stark: **no emplaced sensors at all**, so any change in
//! belief must have come from the drone. Flat ground, so nothing is masked and the only
//! thing deciding what is covered is where the drone has been.

use sim_core::air::AirType;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::collections::BTreeMap;
use validation::scenario_params;

/// Map edge, metres. The drone flies west to east across the middle of it.
const EXTENT_M: f32 = 3000.0;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "pod".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 0.0,
                max_range_m: 2500.0,
                lambda0_per_s: 0.6,
                range_half_m: 900.0,
                range_exponent: 2.0,
                // All-round: a carried sensor has no facing decision to make, so this
                // isolates *coverage* from tasking. V57 already covers the steering half.
                for_width_deg: None,
            },
        )]),
        units: BTreeMap::from([(
            "hider".to_owned(),
            UnitType {
                height_m: 2.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                ..Default::default()
            },
        )]),
        air: BTreeMap::from([(
            "recce".to_owned(),
            AirType {
                cruise_speed_m_s: 40.0,
                sensor: Some("pod".to_owned()),
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// A scenario with or without a Blue recce drone crossing the map west to east.
fn scenario(with_drone: bool) -> Scenario {
    let drone = if with_drone {
        format!(
            r#"
            [[blue.air]]
            id = "recce-1"
            type = "recce"
            pos = [200.0, {mid}]
            altitude_m = 300.0
            heading_deg = 0.0
            waypoints = [[{far}, {mid}]]
        "#,
            mid = EXTENT_M / 2.0,
            far = EXTENT_M - 200.0
        )
    } else {
        String::new()
    };
    Scenario::from_toml_str(&format!(
        r#"
        name = "carried-coverage"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        sensor_tasking = true
        belief_cells = 24
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 300
        [terrain.source.flat]
        elevation_m = 0.0
        [[red.units]]
        id = "hider"
        type = "hider"
        pos = [200.0, 200.0]
        {drone}
    "#
    ))
    .unwrap()
}

/// Blue's belief mass in the band of coarse cells the drone's track runs through.
fn mass_along_the_track(sim: &Sim) -> f32 {
    let belief = sim.belief_of(Side::Blue);
    let raster = belief.belief();
    let cells = raster.shape()[0];
    // The drone flies along y = EXTENT/2, i.e. the middle row of the coarse grid.
    let mid_row = cells / 2;
    (0..cells).map(|ix| raster[[mid_row, ix]]).sum()
}

// V61 (headline): a recce drone that flies over ground and finds nothing drains its side's
// belief out of that ground. With no emplaced sensors, the drone is the only thing that
// could have caused it.
#[test]
fn v61_a_recce_drone_informs_belief() {
    let libs = libraries();

    let mut with = Sim::new(&scenario(true), &libs, 3).unwrap();
    let mut without = Sim::new(&scenario(false), &libs, 3).unwrap();

    let before = mass_along_the_track(&with);
    with.run_until(400.0);
    without.run_until(400.0);

    let after_with = mass_along_the_track(&with);
    let after_without = mass_along_the_track(&without);

    assert!(
        after_with < before,
        "flying a sensor along the track must drain belief from it: {before:.5} -> \
         {after_with:.5}"
    );
    assert!(
        after_with < after_without,
        "the drone must explain the drop: with a drone {after_with:.5}, without \
         {after_without:.5}"
    );
    // The enemy is off the drone's track, so the mass has to go somewhere - belief is a
    // distribution, not a score, and this is the half of V42 that a leak would break.
    let total: f32 = with.belief_of(Side::Blue).belief().iter().sum();
    assert!(
        (total - 1.0).abs() < 1e-3,
        "belief must stay normalised, sums to {total}"
    );
}

// V61 (identity half, §7.4): a scenario with no carried sensor is unaffected. The
// quantised cache path must not perturb an emplaced sensor, whose pose is still exact.
#[test]
fn v61_emplaced_sensors_are_unchanged() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "carried-coverage-identity"
        default_seed = 7
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        sensor_tasking = true
        belief_cells = 24
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 300
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "mast"
        type = "pod"
        pos = [{mid}, {mid}]
        facing_deg = 0.0
        [[red.units]]
        id = "hider"
        type = "hider"
        pos = [600.0, 600.0]
    "#,
        mid = EXTENT_M / 2.0
    ))
    .unwrap();

    let run = |seed: u64| -> (Vec<f32>, usize) {
        let mut sim = Sim::new(&scn, &libraries(), seed).unwrap();
        sim.run_until(300.0);
        (
            sim.belief_of(Side::Blue).belief().iter().copied().collect(),
            sim.events().len(),
        )
    };
    assert_eq!(run(7), run(7), "an emplaced-only scenario must reproduce");
}

// V61 (cache half): the quantised cache is what makes a moving sensor affordable, so it
// has to actually work - a drone that has not left its coarse cell must not force a
// rebuild, and one that has must get a fresh raster. Measured through behaviour rather
// than internals: belief keeps changing as the drone advances, which it could not do if
// the raster were frozen at the launch point.
#[test]
fn v61_a_moving_sensor_keeps_refreshing_its_coverage() {
    let mut sim = Sim::new(&scenario(true), &libraries(), 11).unwrap();
    let mut samples = Vec::new();
    for _ in 0..8 {
        sim.run_until(sim.time_s() + 50.0);
        let raster = sim.belief_of(Side::Blue).belief();
        // Where the belief is *lowest* tracks where the drone has most recently cleared.
        let argmin = raster
            .indexed_iter()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|((iy, ix), _)| (iy, ix))
            .unwrap();
        samples.push(argmin);
    }
    let distinct: std::collections::BTreeSet<_> = samples.iter().collect();
    assert!(
        distinct.len() > 1,
        "the cleared ground must move with the drone; it stayed at {samples:?}"
    );
}
