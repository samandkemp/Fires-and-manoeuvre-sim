//! Scenario and stat-block loading, and the guarantee that every shipped scenario
//! resolves against the libraries (docs/DESIGN.md §1.3).

use sim_core::scenario::*;
use sim_core::terrain::TerrainType;
use std::path::Path;
use validation::scenario_path;

#[test]
fn loads_default_scenario() {
    let scn = Scenario::load(&scenario_path("default.toml")).expect("default should load");
    assert_eq!(scn.name, "default");
    assert_eq!(scn.terrain.cell_size_m, 10.0);
    assert!(scn.terrain.width_cells > 0 && scn.terrain.height_cells > 0);
}

#[test]
fn default_scenario_paints_all_terrain_types() {
    let scn = Scenario::load(&scenario_path("default.toml")).unwrap();
    let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
    let g = scn.build_terrain(&params, scn.default_seed);
    let trees = g
        .terrain_type()
        .iter()
        .filter(|&&t| t == TerrainType::Trees)
        .count();
    let urban = g
        .terrain_type()
        .iter()
        .filter(|&&t| t == TerrainType::Urban)
        .count();
    // Diagnostic bounds, visible with `cargo test -- --nocapture`.
    let mut bounds = (usize::MAX, 0usize, usize::MAX, 0usize);
    for ((iy, ix), &t) in g.terrain_type().indexed_iter() {
        if t == TerrainType::Urban {
            bounds.0 = bounds.0.min(ix);
            bounds.1 = bounds.1.max(ix);
            bounds.2 = bounds.2.min(iy);
            bounds.3 = bounds.3.max(iy);
        }
    }
    println!(
        "urban cells: {urban} (ix {}..{}, iy {}..{})",
        bounds.0, bounds.1, bounds.2, bounds.3
    );
    assert!(trees > 0, "default scenario should paint woods");
    assert!(urban > 0, "default scenario should paint urban blocks");
}

#[test]
fn loads_flat_fixture_scenario() {
    let scn = Scenario::load(&scenario_path("flat_range.toml")).expect("fixture should load");
    let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();
    let g = scn.build_terrain(&params, scn.default_seed);
    assert!(g.elevation().iter().all(|&z| z == 100.0));
}

// Every scenario shipped in `scenarios/` must parse *and* resolve every instance
// against the libraries — the gate that catches a typo'd type id or a schema drift in
// air.toml / air_defence.toml, which otherwise only shows up when the app won't start.
#[test]
fn shipped_scenarios_load_and_resolve() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let libs = Libraries::load_dir(&dir).expect("libraries should load");
    assert!(
        !libs.air.is_empty() && !libs.air_defence.is_empty(),
        "the air libraries should be present"
    );

    for name in ["default.toml", "flat_range.toml", "air_raid.toml"] {
        let scn =
            Scenario::load(&dir.join(name)).unwrap_or_else(|e| panic!("{name} should load: {e}"));
        let sim = sim_core::sim::Sim::new(&scn, &libs, scn.default_seed)
            .unwrap_or_else(|e| panic!("{name} should resolve: {e}"));
        // A resolved air asset must have picked up whatever its type declared.
        for a in sim.air() {
            assert!(
                a.sensor.is_some() || a.payload.is_some(),
                "{name}: air asset '{}' carries neither sensor nor payload",
                a.id
            );
            assert!(
                a.alive && a.speed_m_s > 0.0,
                "{name}: '{}' cannot fly",
                a.id
            );
        }
    }
}

// The air raid scenario is the counter-air demo: it must actually produce a fight.
#[test]
fn air_raid_scenario_produces_a_counter_air_fight() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let libs = Libraries::load_dir(&dir).unwrap();
    let scn = Scenario::load(&dir.join("air_raid.toml")).unwrap();
    let mut sim = sim_core::sim::Sim::new(&scn, &libs, scn.default_seed).unwrap();
    assert_eq!(sim.air().len(), 4);
    assert_eq!(sim.air_defence().len(), 2);
    sim.run_until(400.0);
    assert!(
        !sim.air_events().is_empty(),
        "Blue should detect the inbound raid"
    );
    assert!(
        !sim.air_defence_events().is_empty(),
        "and the defences should engage it"
    );
}

#[test]
fn loads_terrain_params() {
    let table =
        load_terrain_params(&scenario_path("terrain_types.toml")).expect("params should load");
    assert!(table.get(TerrainType::Trees).extinction_per_m > 0.0);
    assert_eq!(table.get(TerrainType::Open).mobility_cost, 1.0);
}

#[test]
fn rejects_zero_dimensions() {
    let bad = r#"
        name = "bad"
        [terrain]
        cell_size_m = 10.0
        width_cells = 0
        height_cells = 10
        [terrain.source.flat]
        elevation_m = 0.0
    "#;
    assert!(matches!(
        Scenario::from_toml_str(bad),
        Err(ScenarioError::Invalid(_))
    ));
}

#[test]
fn rejects_malformed_toml() {
    assert!(matches!(
        Scenario::from_toml_str("this is = not valid ]"),
        Err(ScenarioError::Parse(_))
    ));
}

#[test]
fn missing_file_is_io_error() {
    let err = Scenario::load(Path::new("definitely-not-here-42.toml")).unwrap_err();
    assert!(matches!(err, ScenarioError::Io { .. }));
}

#[test]
fn builds_terrain_deterministically_from_scenario() {
    let text = r#"
        name = "tiny"
        default_seed = 3
        [terrain]
        cell_size_m = 5.0
        width_cells = 32
        height_cells = 20
        [terrain.source.hills]
        count = 5
        max_height_m = 50.0
        base_radius_m = 40.0
    "#;
    let scn = Scenario::from_toml_str(text).expect("tiny scenario should parse");
    let params = load_terrain_params(&scenario_path("terrain_types.toml")).unwrap();

    let g1 = scn.build_terrain(&params, scn.default_seed);
    assert_eq!(g1.width(), 32);
    assert_eq!(g1.height(), 20);

    let g2 = scn.build_terrain(&params, scn.default_seed);
    assert_eq!(
        g1.elevation(),
        g2.elevation(),
        "same seed must reproduce terrain"
    );
}

// --- V67: the input contract (docs/DESIGN.md §7.6) ------------------------------------
//
// `deny_unknown_fields` already refuses a key the schema does not know. These refuse a
// *value* the model cannot run on. The two clock dials are the ones that matter: they do
// not give a wrong answer, they fail to terminate, and both are reachable from an
// ordinary-looking `sweep --param sim.epoch_s --from 0`.

/// A minimal valid scenario with `line` spliced into its `[sim]` block.
fn with_sim_dial(line: &str) -> Result<Scenario, ScenarioError> {
    Scenario::from_toml_str(&format!(
        r#"
        name = "dial"
        default_seed = 1
        [sim]
        {line}
        [terrain]
        cell_size_m = 10.0
        width_cells = 8
        height_cells = 8
        [terrain.source.flat]
        elevation_m = 0.0
    "#
    ))
}

#[test]
fn v67_a_dial_that_would_not_terminate_is_refused_at_load() {
    // `dt_s = 0` never advances the clock, so `run_until` never returns.
    // `epoch_s = 0` makes `time_s / epoch_s` infinite; the cast to u64 saturates rather
    // than wrapping, so the epoch loop is handed u64::MAX boundaries and hangs.
    for bad in [
        "dt_s = 0.0",
        "dt_s = -1.0",
        "epoch_s = 0.0",
        "epoch_s = -10.0",
    ] {
        let err = with_sim_dial(bad).expect_err("a non-terminating dial must be refused");
        let msg = format!("{err}");
        assert!(
            matches!(err, ScenarioError::Invalid(_)),
            "{bad} should be a validation error, got {msg}"
        );
        // The message has to name the dial, or it cannot be acted on.
        let dial = bad.split(' ').next().unwrap();
        assert!(
            msg.contains(dial),
            "{bad}: message must name the dial: {msg}"
        );
    }
    // The good values still load, so the check is a floor and not a wall.
    assert!(with_sim_dial("dt_s = 0.25\nepoch_s = 2.5").is_ok());
}

#[test]
fn v67_a_dial_outside_its_domain_is_refused_at_load() {
    for bad in [
        "p_suppress = 1.5",
        "p_suppress = -0.1",
        "track_maintain_p = 2.0",
        "suppressed_fire_factor = -1.0",
        "track_hold_s = -5.0",
        "recover_per_s = -0.1",
        "suppression_radius_m = -1.0",
        "belief_cells = 0",
    ] {
        let err = with_sim_dial(bad).expect_err("should be refused");
        let dial = bad.split(' ').next().unwrap();
        assert!(
            format!("{err}").contains(dial),
            "{bad}: message must name the dial: {err}"
        );
    }
    // A probability at either end of its range is legitimate and must still load.
    assert!(with_sim_dial("p_suppress = 0.0\ntrack_maintain_p = 1.0").is_ok());
}

#[test]
fn v67_a_stat_block_that_would_evaluate_to_nan_is_refused() {
    use sim_core::fires::{WeaponClass, WeaponType};
    use sim_core::sensing::{Modality, SensorType};
    use std::collections::BTreeMap;

    let sensor = |range_half_m: f32| SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 4000.0,
        lambda0_per_s: 0.5,
        range_half_m,
        range_exponent: 2.0,
        for_width_deg: None,
    };
    let base = Libraries::with_terrain(validation::scenario_params());

    // `range_half_m` divides the §3.2 falloff: at zero the rate is NaN, and a NaN rate
    // loses `rng < p`, so the sensor silently never detects anything.
    let libs = Libraries {
        sensors: BTreeMap::from([("blind".to_owned(), sensor(0.0))]),
        ..base.clone()
    };
    let err = libs
        .validate()
        .expect_err("range_half_m = 0 must be refused");
    assert!(
        format!("{err}").contains("sensors.blind.range_half_m"),
        "the message must name the library, the block and the dial: {err}"
    );
    // The same library with a workable falloff passes, so this is a floor not a wall.
    let ok = Libraries {
        sensors: BTreeMap::from([("fine".to_owned(), sensor(1200.0))]),
        ..base.clone()
    };
    assert!(ok.validate().is_ok());

    // The Carleton kernel divides by 2·R_L², so an *indirect* weapon needs a lethal
    // radius. A direct weapon does not use the kernel at all and keeps its zero.
    let indirect = |lethal_radius_m: f32| WeaponType {
        class: WeaponClass::Indirect,
        rof_rounds_per_min: 6.0,
        max_range_m: 8000.0,
        cep_m: 40.0,
        lethal_radius_m,
        ..Default::default()
    };
    let libs = Libraries {
        weapons: BTreeMap::from([("mortar".to_owned(), indirect(0.0))]),
        ..base.clone()
    };
    assert!(format!("{}", libs.validate().expect_err("must be refused"))
        .contains("weapons.mortar.lethal_radius_m"));
    let libs = Libraries {
        weapons: BTreeMap::from([
            ("mortar".to_owned(), indirect(40.0)),
            ("cannon".to_owned(), WeaponType::default()), // Direct, lethal_radius 0: fine
        ]),
        ..base
    };
    assert!(
        libs.validate().is_ok(),
        "a direct weapon never touches the kernel, so its zero is legitimate"
    );
}

#[test]
fn v67_every_shipped_scenario_and_library_passes_its_own_contract() {
    // The check is only worth having if the repository satisfies it — otherwise it would
    // have to be weakened the first time it was run in anger.
    let dir = validation::scenarios_dir();
    Libraries::load_dir(&dir).expect("the shipped libraries must satisfy the contract");
    for entry in std::fs::read_dir(&dir).expect("scenarios dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "toml") {
            // Libraries are not scenarios; only judge the files that parse as one.
            if let Err(ScenarioError::Invalid(msg)) = Scenario::load(&path) {
                panic!("{} fails the input contract: {msg}", path.display());
            }
        }
    }
}
