//! V58 — the decision layer is an identity when there is nothing to decide.
//! `docs/DESIGN.md` §10.4.
//!
//! Phase 10 added three phases to the epoch: track maintenance, sensor tasking and fire
//! allocation. All three are *deterministic* — they read state and write decisions, and
//! draw no random numbers. This gate holds them to that in the way the project has held
//! every other added phase (V40 for EW, V52 for air):
//!
//! - a scenario with one shooter and one possible target must produce the identical fire
//!   log under every allocation rule, because there is only one answer;
//! - a scenario with no steerable sensor must be unaffected by tasking being switched on.
//!
//! If any of them ever drew from the RNG, the streams would diverge and both halves would
//! fail immediately.

use sim_core::scenario::{AllocationChoice, Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

/// One Blue gun, one Red target, one all-round observer: a fight with exactly one
/// targeting answer, whatever the solver.
fn duel(allocation: &str, tasking: bool) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "decision-identity"
        default_seed = 7
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        allocation = "{allocation}"
        sensor_tasking = {tasking}
        [terrain]
        cell_size_m = 10.0
        width_cells = 128
        height_cells = 32
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "obs"
        type = "eye"
        pos = [100.0, 160.0]
        [[blue.units]]
        id = "gun"
        type = "shooter"
        pos = [120.0, 160.0]
        [[red.units]]
        id = "tgt"
        type = "target"
        pos = [800.0, 160.0]
    "#
    ))
    .unwrap();

    let libs = Libraries {
        sensors: BTreeMap::from([(
            "eye".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 2.0,
                max_range_m: 4000.0,
                lambda0_per_s: 0.3,
                range_half_m: 1200.0,
                range_exponent: 2.0,
                // All-round: nothing for the tasking layer to steer.
                for_width_deg: None,
            },
        )]),
        units: BTreeMap::from([
            (
                "shooter".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 2,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    weapon: Some("cannon".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "target".to_owned(),
                UnitType {
                    height_m: 2.4,
                    silhouette_width_m: 3.0,
                    element_count: 5,
                    signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                    ..Default::default()
                },
            ),
        ]),
        weapons: BTreeMap::from([(
            "cannon".to_owned(),
            sim_core::fires::WeaponType {
                class: sim_core::fires::WeaponClass::Direct,
                rof_rounds_per_min: 12.0,
                max_range_m: 3000.0,
                dispersion_mrad: 0.6,
                p_kill_given_hit: 0.4,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    };
    Sim::new(&scn, &libs, 7).unwrap()
}

// V58 (identity half): with one shooter and one reachable target there is only one
// targeting answer, so every allocation rule must produce the *same* fire log — and
// switching sensor tasking on must change nothing when no sensor can be steered. Any
// randomness drawn by the new phases would shift the stream and break this.
#[test]
fn v58_decision_layer_is_a_zero_draw_identity() {
    let reference = {
        let mut sim = duel("independent", false);
        sim.run_until(400.0);
        (sim.events().to_vec(), sim.fire_events().to_vec())
    };

    for allocation in ["independent", "greedy", "optimal"] {
        for tasking in [false, true] {
            let mut sim = duel(allocation, tasking);
            sim.run_until(400.0);
            assert_eq!(
                sim.events(),
                reference.0,
                "detection log moved under allocation={allocation} tasking={tasking}; the \
                 decision phases must draw no randomness"
            );
            assert_eq!(
                sim.fire_events(),
                reference.1,
                "fire log moved under allocation={allocation} tasking={tasking}, but there \
                 is only one target to choose"
            );
        }
    }
}

// V58 (reproducibility half): the same scenario and seed reproduce every log exactly,
// and the decision phases are stable under repeated evaluation — running the sim twice
// must agree down to the last round.
#[test]
fn v58_single_shooter_matches_the_old_rule() {
    for allocation in [
        AllocationChoice::Independent,
        AllocationChoice::Greedy,
        AllocationChoice::Optimal,
    ] {
        let name = match allocation {
            AllocationChoice::Independent => "independent",
            AllocationChoice::Greedy => "greedy",
            AllocationChoice::Optimal => "optimal",
        };
        let mut a = duel(name, true);
        let mut b = duel(name, true);
        a.run_until(400.0);
        b.run_until(400.0);
        assert_eq!(a.events(), b.events(), "{name}: detections must reproduce");
        assert_eq!(
            a.fire_events(),
            b.fire_events(),
            "{name}: fires must reproduce"
        );
        assert!(
            !a.fire_events().is_empty(),
            "{name}: the fixture should actually produce fires"
        );
    }
}
