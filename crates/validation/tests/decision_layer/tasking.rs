//! V57 - belief-driven sensor tasking. `docs/DESIGN.md` §10.3.
//!
//! The claim under test is that pointing a sensor by belief beats leaving it pointed
//! where it started. The fixture makes that measurable: a narrow-arc sensor in the
//! middle of open ground, and one hidden enemy placed on a bearing the sensor is *not*
//! initially covering.
//!
//! A fixed stare can only ever find an enemy that walks into its arc. A belief-driven
//! sensor drains its own belief out of the ground it has already cleared, so the
//! best-information facing moves on - the sweep is not scripted, it falls out of
//! maximising expected entropy reduction.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::collections::BTreeMap;
use validation::scenario_params;

/// Sensor at the centre of a 2 x 2 km field, arc 60° wide, initially facing east (0°).
const CENTRE: f32 = 1000.0;
/// How far out the hidden enemy sits - close enough to be found quickly once looked at.
const RANGE_M: f32 = 700.0;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "narrow".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 2.0,
                max_range_m: 4000.0,
                lambda0_per_s: 0.5,
                range_half_m: 1200.0,
                range_exponent: 2.0,
                // The whole point: a sensor that cannot see everywhere at once has a
                // decision to make. An all-round sensor would have nothing to task.
                for_width_deg: Some(60.0),
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
        ..Libraries::with_terrain(scenario_params())
    }
}

/// One trial: a hidden unit at `bearing_deg` from the sensor, which starts facing east.
/// Returns the time of first detection, or `horizon` if it was never found.
fn time_to_detect(bearing_deg: f32, tasking: bool, seed: u64, horizon: f64) -> f64 {
    let (dx, dy) = (
        bearing_deg.to_radians().cos() * RANGE_M,
        bearing_deg.to_radians().sin() * RANGE_M,
    );
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "tasking"
        default_seed = 1
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        sensor_tasking = {tasking}
        belief_cells = 32
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "eye"
        type = "narrow"
        pos = [{CENTRE}, {CENTRE}]
        facing_deg = 0.0
        [[red.units]]
        id = "hider"
        type = "hider"
        pos = [{}, {}]
    "#,
        CENTRE + dx,
        CENTRE + dy
    ))
    .unwrap();

    let mut sim = Sim::new(&scn, &libraries(), seed).unwrap();
    sim.run_until(horizon);
    sim.events().first().map_or(horizon, |e| e.time_s)
}

// V57 (headline): over enemies hidden on bearings the sensor does not start on, a
// belief-driven sensor finds them and a fixed stare does not.
#[test]
fn v57_tasking_beats_a_fixed_stare() {
    // Bearings well outside the initial 60°-wide arc centred on 0°.
    let bearings = [90.0f32, 135.0, 180.0, 225.0, 270.0];
    let horizon = 600.0;

    let mut tasked_total = 0.0;
    let mut fixed_total = 0.0;
    let mut tasked_found = 0;
    let mut fixed_found = 0;

    for &bearing in &bearings {
        for seed in 0..6u64 {
            let tasked = time_to_detect(bearing, true, seed, horizon);
            let fixed = time_to_detect(bearing, false, seed, horizon);
            tasked_total += tasked;
            fixed_total += fixed;
            tasked_found += usize::from(tasked < horizon);
            fixed_found += usize::from(fixed < horizon);
        }
    }
    let n = (bearings.len() * 6) as f64;

    assert_eq!(
        fixed_found, 0,
        "a fixed stare must not find an enemy outside its arc - the fixture is wrong \
         if it does"
    );
    assert!(
        tasked_found > 0,
        "belief-driven tasking found nothing in {} trials; the sensor is not slewing",
        n as usize
    );
    assert!(
        tasked_total / n < fixed_total / n,
        "tasking mean {:.1} s should beat a fixed stare's {:.1} s",
        tasked_total / n,
        fixed_total / n
    );
}

// V57 (belief half, extending V42): the belief stays a proper distribution for the whole
// run - non-negative, normalised, and no NaN - however many times it is updated and
// diffused.
#[test]
fn v57_belief_stays_a_proper_distribution() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "tasking-belief"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        sensor_tasking = true
        belief_cells = 24
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "eye"
        type = "narrow"
        pos = [{CENTRE}, {CENTRE}]
        facing_deg = 0.0
        [[red.units]]
        id = "hider"
        type = "hider"
        pos = [300.0, 300.0]
    "#
    ))
    .unwrap();
    let mut sim = Sim::new(&scn, &libraries(), 4).unwrap();

    for _ in 0..40 {
        sim.run_until(sim.time_s() + 10.0);
        for side in [Side::Blue, Side::Red] {
            let belief = sim.belief_of(side);
            let raster = belief.belief();
            let sum: f32 = raster.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-3,
                "belief for {side:?} must stay normalised, sums to {sum}"
            );
            assert!(
                raster.iter().all(|p| p.is_finite() && *p >= 0.0),
                "belief for {side:?} must stay non-negative and finite"
            );
            assert!(
                belief.entropy().is_finite(),
                "entropy for {side:?} must stay finite"
            );
        }
    }
}

// V57 (determinism half): tasking draws no randomness, so the same scenario and seed
// must reproduce both the chosen facings and the detection log exactly.
#[test]
fn v57_tasking_is_deterministic() {
    let facings = |seed: u64| -> Vec<f32> {
        let mut sim = {
            let scn = Scenario::from_toml_str(&format!(
                r#"
                name = "tasking-determinism"
                default_seed = {seed}
                [sim]
                dt_s = 1.0
                epoch_s = 10.0
                sensor_tasking = true
                belief_cells = 24
                [terrain]
                cell_size_m = 10.0
                width_cells = 200
                height_cells = 200
                [terrain.source.flat]
                elevation_m = 0.0
                [[blue.sensors]]
                id = "eye"
                type = "narrow"
                pos = [{CENTRE}, {CENTRE}]
                facing_deg = 0.0
                [[red.units]]
                id = "hider"
                type = "hider"
                pos = [400.0, 1600.0]
            "#
            ))
            .unwrap();
            Sim::new(&scn, &libraries(), seed).unwrap()
        };
        let mut seen = Vec::new();
        for _ in 0..12 {
            sim.run_until(sim.time_s() + 10.0);
            seen.push(sim.sensors()[0].facing_deg);
        }
        seen
    };
    assert_eq!(facings(9), facings(9), "tasking must be reproducible");
}
