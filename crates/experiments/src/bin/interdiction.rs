//! The interdiction game (Phase 6 capstone). Blue chooses an overwatch *position*
//! (sensor + co-located observed-indirect mortar); Red chooses a *route* across the map.
//! The zero-sum payoff is Red's expected attrition as it traverses while Blue detects and
//! bombards it — estimated by short headless Monte-Carlo battles. Fictitious play then
//! solves for the equilibrium: Blue's optimal mixed placement, Red's optimal mixed route,
//! and the game value (expected attrition).
//!
//! Run: `cargo run -p experiments --release --bin interdiction`

use glam::Vec2;
use ndarray::Array2;
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::game::solve_zero_sum;
use sim_core::scenario::Scenario;
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::collections::BTreeMap;

const SEEDS: u64 = 60; // Monte-Carlo battles per matrix cell
const RED_SPEED: f32 = 10.0;

fn main() {
    // A 3 km map with relief and woods for LOS masking (built once, reused per battle).
    let scn = Scenario::from_toml_str(
        r#"
        name = "interdiction"
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 300
        [terrain.source.hills]
        count = 10
        max_height_m = 25.0
        base_radius_m = 300.0
        woods_fraction = 0.15
        urban_blocks = 1
    "#,
    )
    .unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let terrain_params =
        sim_core::scenario::load_terrain_params(&dir.join("terrain_types.toml")).unwrap();

    // Stat blocks (inline placeholders for the game).
    // A sector observer: a limited field of regard, so each overwatch covers one lane's
    // approach and not the others — the geometry that makes this a hide-and-seek game.
    let sensor = SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 2500.0,
        lambda0_per_s: 0.7,
        range_half_m: 1000.0,
        range_exponent: 2.0,
        for_width_deg: Some(55.0),
    };
    let mortar_unit = UnitType {
        height_m: 2.0,
        element_count: 1,
        signature: BTreeMap::new(),
        weapon: Some("mortar".to_owned()),
        ..Default::default()
    };
    let mortar = WeaponType {
        class: WeaponClass::Indirect,
        rof_rounds_per_min: 12.0,
        max_range_m: 4000.0,
        cep_m: 50.0,
        lethal_radius_m: 35.0,
        ..Default::default()
    };
    let red_unit = UnitType {
        height_m: 2.8,
        silhouette_width_m: 3.2,
        element_count: 4,
        speed_m_s: RED_SPEED,
        signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
        weapon: None, // pure interdiction target
    };

    // Three lanes (south / centre / north). Each Blue overwatch sits mid-lane facing
    // west (180°) to watch that lane's western approach; each Red route runs the lane
    // west → east. Blue picks a lane to watch, Red a lane to cross — hide-and-seek.
    let lanes = [("south", 750.0f32), ("centre", 1500.0), ("north", 2250.0)];
    let blue_sites: Vec<(&str, Vec2, f32)> = lanes
        .iter()
        .map(|&(n, y)| (n, Vec2::new(1500.0, y), 180.0f32))
        .collect();
    let red_routes: Vec<(&str, Vec<Vec2>)> = lanes
        .iter()
        .map(|&(n, y)| (n, vec![Vec2::new(100.0, y), Vec2::new(2900.0, y)]))
        .collect();

    // Build the sim once (terrain-only), then reset + repopulate per battle.
    let libs = sim_core::scenario::Libraries::with_terrain(terrain_params);
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();

    let (nb, nr) = (blue_sites.len(), red_routes.len());
    let mut payoff = Array2::<f32>::zeros((nb, nr));
    let t0 = std::time::Instant::now();
    for (bi, (_, b, facing)) in blue_sites.iter().enumerate() {
        for (ri, (_, route)) in red_routes.iter().enumerate() {
            let mut attrition_sum = 0.0f32;
            for seed in 0..SEEDS {
                attrition_sum += one_battle(
                    &mut sim,
                    *b,
                    *facing,
                    &sensor,
                    &mortar_unit,
                    &mortar,
                    route,
                    &red_unit,
                    seed,
                );
            }
            payoff[[bi, ri]] = attrition_sum / SEEDS as f32;
        }
    }
    eprintln!(
        "payoff matrix ({nb}x{nr}, {SEEDS} seeds/cell) built in {:?}",
        t0.elapsed()
    );

    // Show the matrix.
    print!("\n              ");
    for (name, _) in &red_routes {
        print!("{name:>11}");
    }
    println!("   (Red route →)");
    for (bi, (bname, _, _)) in blue_sites.iter().enumerate() {
        print!("  {bname:>11} ");
        for ri in 0..nr {
            print!("{:>11.2}", payoff[[bi, ri]]);
        }
        println!();
    }

    // Solve the zero-sum game (Blue maximises attrition, Red minimises).
    let sol = solve_zero_sum(&payoff, 200_000);
    println!(
        "\nEquilibrium (fictitious play), value = {:.3} expected Red attrition:",
        sol.value
    );
    println!("  Blue mixed overwatch:");
    for (i, (name, _, _)) in blue_sites.iter().enumerate() {
        if sol.row_strategy[i] > 0.005 {
            println!("    {name:>11}: {:>5.1}%", 100.0 * sol.row_strategy[i]);
        }
    }
    println!("  Red mixed route:");
    for (j, (name, _)) in red_routes.iter().enumerate() {
        if sol.col_strategy[j] > 0.005 {
            println!("    {name:>11}: {:>5.1}%", 100.0 * sol.col_strategy[j]);
        }
    }
    println!("  (value bracket gap {:.4})", sol.value_gap);
}

/// One battle: Red traverses `route` while Blue at `b` (sensor + mortar) interdicts.
/// Returns Red's attrition fraction (elements lost / initial).
#[allow(clippy::too_many_arguments)]
fn one_battle(
    sim: &mut Sim,
    b: Vec2,
    facing_deg: f32,
    sensor: &SensorType,
    mortar_unit: &UnitType,
    mortar: &WeaponType,
    route: &[Vec2],
    red_unit: &UnitType,
    seed: u64,
) -> f32 {
    sim.reset(seed);
    sim.add_sensor("obs", Side::Blue, b, facing_deg, sensor.clone());
    sim.add_unit(
        "mortar",
        Side::Blue,
        b,
        mortar_unit.clone(),
        Some(mortar.clone()),
    );
    sim.add_unit("red", Side::Red, route[0], red_unit.clone(), None);
    let red_idx = sim.units().len() - 1;
    sim.set_route(red_idx, route.to_vec());

    // Run until Red finishes the route or is destroyed (with a safety time cap).
    loop {
        sim.step_one();
        let red = &sim.units()[red_idx];
        let finished = red.route_idx >= red.route.len();
        if !red.alive() || finished || sim.time_s() > 800.0 {
            return 1.0 - red.strength();
        }
    }
}
