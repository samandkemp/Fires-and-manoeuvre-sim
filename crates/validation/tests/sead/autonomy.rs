//! V76 - autonomous strike targeting. `docs/THEORY.md` §9.3.
//!
//! A strike drone attacked the asset its scenario named and nothing else, even when its own
//! side had found something better on the way. The same limitation was recorded independently
//! on three design pages, which is the clearest signal in the document set that it was the
//! real gap rather than a local irritation.
//!
//! Three properties, and the first is the one that keeps every existing result standing.

use sim_core::airframes::AirType;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::Sim;
use sim_core::weapon_effects::{WeaponClass, WeaponType};
use std::collections::BTreeMap;
use validation::scenario_params;

fn libraries(autonomous: bool) -> Libraries {
    let mut libs = Libraries::with_terrain(scenario_params());
    libs.weapons.insert(
        "bomblet".to_owned(),
        WeaponType {
            class: WeaponClass::Indirect,
            max_range_m: 400.0,
            cep_m: 10.0,
            lethal_radius_m: 60.0,
            p_kill_given_hit: 0.9,
            rof_rounds_per_min: 30.0,
            ..Default::default()
        },
    );
    libs.units.insert(
        "target".to_owned(),
        UnitType {
            height_m: 2.5,
            silhouette_width_m: 3.0,
            element_count: 2,
            signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
            ..Default::default()
        },
    );
    libs.sensors.insert(
        "eye".to_owned(),
        SensorType {
            modality: Modality::Optical,
            mount_height_m: 20.0,
            max_range_m: 6000.0,
            lambda0_per_s: 4.0,
            range_half_m: 4000.0,
            range_exponent: 2.0,
            for_width_deg: None,
        },
    );
    libs.air.insert(
        "striker".to_owned(),
        AirType {
            height_m: 1.0,
            cruise_speed_m_s: 40.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.4)]),
            payload: Some("bomblet".to_owned()),
            munitions: 2,
            release_range_m: 500.0,
            autonomous,
            ..Default::default()
        },
    );
    libs
}

/// Blue flies a striker west to east past two Red units. Its assigned target is a map point
/// beyond them, so the assignment is satisfiable and never expires - which is what makes the
/// "assignment still wins" case testable.
fn scenario(assigned: Option<&str>) -> Scenario {
    let target_line = match assigned {
        Some(spec) => format!("target = {spec}"),
        None => String::new(),
    };
    Scenario::from_toml_str(&format!(
        r#"
        name = "autonomy"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "eye"
        type = "eye"
        pos = [200.0, 1000.0]
        [[blue.air]]
        id = "striker"
        type = "striker"
        pos = [200.0, 1000.0]
        altitude_m = 150.0
        heading_deg = 0.0
        waypoints = [[200.0, 1000.0], [3600.0, 1000.0]]
        {target_line}
        [[red.units]]
        id = "near"
        type = "target"
        pos = [1400.0, 1000.0]
        [[red.units]]
        id = "far"
        type = "target"
        pos = [2600.0, 1000.0]
    "#
    ))
    .expect("fixture parses")
}

/// Where every munition was aimed, which is what actually distinguishes the behaviours: a
/// drone with no assignment has always aimed at its flight-plan destination, so a count of
/// strikes cannot tell "ignored the enemy" from "attacked the enemy" at all.
fn aim_points(autonomous: bool, assigned: Option<&str>) -> Vec<f32> {
    let scn = scenario(assigned);
    let Ok(mut sim) = Sim::new(&scn, &libraries(autonomous), 4) else {
        return Vec::new();
    };
    sim.run_until(200.0);
    sim.strike_events().iter().map(|e| e.aim.x).collect()
}

/// Did anything get aimed at a Red unit rather than at the far end of the route?
fn struck_an_enemy(aims: &[f32]) -> bool {
    aims.iter()
        .any(|&x| (x - 1400.0).abs() < 50.0 || (x - 2600.0).abs() < 50.0)
}

// The identity. An airframe that is not autonomous - which is every airframe in every
// scenario written before this existed - releases only on an assigned target, so a drone
// flying past two enemies with nothing assigned must still drop nothing.
#[test]
fn v76_without_autonomy_a_drone_ignores_what_it_passes() {
    let aims = aim_points(false, None);
    assert!(
        !aims.is_empty(),
        "the fixture must release something, or the comparison is vacuous"
    );
    assert!(
        !struck_an_enemy(&aims),
        "a non-autonomous drone must aim only at its flight plan's destination, not at the enemies it flies past; aimed at {aims:?}"
    );
}

// The capability. The same flight, the same enemies, the same seed - only the flag differs.
#[test]
fn v76_an_autonomous_drone_strikes_what_it_finds() {
    let aims = aim_points(true, None);
    assert!(
        struck_an_enemy(&aims),
        "an autonomous drone passing within release range of two located enemies should aim at one of them; aimed at {aims:?}"
    );
}

// Orders are not overridden by opportunity. A drone with a live assigned target goes for
// that one, so autonomy adds behaviour where there was none rather than replacing what a
// scenario asked for.
#[test]
fn v76_an_assigned_target_still_wins() {
    // A point target well beyond both Red units: reached last, so if opportunism were
    // overriding the assignment the drone would have struck earlier and elsewhere.
    let aims = aim_points(true, Some("{ point = [3500.0, 1000.0] }"));
    assert!(
        !struck_an_enemy(&aims),
        "an assigned point target must not be abandoned for a closer enemy; aimed at {aims:?}"
    );
}
