//! Isolates the ground-fires path: many shooters, many rounds, no air, minimal sensing.
//! Public API only, so it compiles against the pre- and post-refactor engine alike.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sim::Sim;
use std::time::Instant;

fn scenario(class: &str) -> String {
    let mut s = String::from(
        r#"
name = "fires-bench"
default_seed = 7
[sim]
dt_s = 1.0
epoch_s = 10.0
[terrain]
cell_size_m = 10.0
width_cells = 256
height_cells = 256
[terrain.source.flat]
elevation_m = 0.0
"#,
    );
    // Blue shooters west, Red targets east: in range, in LOS, on flat ground.
    for i in 0..12 {
        s.push_str(&format!(
            "[[blue.units]]\nid = \"b{i}\"\ntype = \"{class}\"\npos = [400.0, {}.0]\n",
            200 + i * 60
        ));
        s.push_str(&format!(
            "[[red.units]]\nid = \"r{i}\"\ntype = \"tgt\"\npos = [1600.0, {}.0]\n",
            200 + i * 60
        ));
        // A sensor per shooter so indirect fire has the tracks it needs.
        s.push_str(&format!(
            "[[blue.sensors]]\nid = \"s{i}\"\ntype = \"eye\"\npos = [400.0, {}.0]\n",
            200 + i * 60
        ));
    }
    s
}

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let base = Libraries::load_dir(&dir).unwrap();

    for (label, class) in [("direct", "gun"), ("indirect", "mortar")] {
        let scn = Scenario::from_toml_str(&scenario(class)).unwrap();
        let mut libs = base.clone();
        // Big element counts so the fight lasts the whole measurement rather than
        // ending early and timing an empty loop.
        libs.units.insert(
            "tgt".to_owned(),
            sim_core::sensing::UnitType {
                height_m: 2.4,
                silhouette_width_m: 3.0,
                element_count: 250,
                signature: std::collections::BTreeMap::from([("optical".to_owned(), 0.9)]),
                ..Default::default()
            },
        );
        libs.units.insert(
            class.to_owned(),
            sim_core::sensing::UnitType {
                height_m: 2.4,
                element_count: 40,
                signature: std::collections::BTreeMap::from([("optical".to_owned(), 0.5)]),
                weapon: Some(format!("{class}_w")),
                ..Default::default()
            },
        );
        libs.weapons.insert(
            format!("{class}_w"),
            if class == "gun" {
                sim_core::fires::WeaponType {
                    class: sim_core::fires::WeaponClass::Direct,
                    rof_rounds_per_min: 60.0,
                    max_range_m: 4000.0,
                    dispersion_mrad: 3.0,
                    p_kill_given_hit: 0.02,
                    ..Default::default()
                }
            } else {
                sim_core::fires::WeaponType {
                    class: sim_core::fires::WeaponClass::Indirect,
                    rof_rounds_per_min: 30.0,
                    max_range_m: 6000.0,
                    cep_m: 60.0,
                    lethal_radius_m: 12.0,
                    ..Default::default()
                }
            },
        );
        libs.sensors.insert(
            "eye".to_owned(),
            sim_core::sensing::SensorType {
                modality: sim_core::sensing::Modality::Optical,
                mount_height_m: 3.0,
                max_range_m: 6000.0,
                lambda0_per_s: 5.0,
                range_half_m: 4000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        );

        let mut sim = Sim::new(&scn, &libs, 7).unwrap();
        let t = Instant::now();
        sim.run_until(3000.0);
        let el = t.elapsed();
        let epochs = sim.epochs_run();
        let casualties: u32 = sim.fire_events().iter().map(|e| e.casualties).sum();
        println!(
            "{label:<9} {el:>9.1?}  {epochs} epochs, {} fire events, {casualties} casualties",
            sim.fire_events().len()
        );
    }
}
