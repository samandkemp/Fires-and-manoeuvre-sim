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
