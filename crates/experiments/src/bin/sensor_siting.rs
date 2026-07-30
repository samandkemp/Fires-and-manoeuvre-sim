//! Sensor-siting sweep — an early taste of the project's whole point. Over a coarse
//! grid of candidate positions for one sensor type, score each by the red units it can
//! actually detect (clear LOS, in range) and the total detection rate it achieves, and
//! print the best sites. A brute-force stand-in for the sensor-placement optimisation
//! Phase 6 will make game-theoretic.
//!
//! Run: `cargo run -p experiments --bin sensor_siting -- [sensor_type] [--release for speed]`

use glam::Vec2;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{detection_rate, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::path::Path;

fn main() {
    let sensor_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "mast_optical".to_owned());

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let scenario = Scenario::load(&dir.join("default.toml")).unwrap();
    let libs = Libraries::load_dir(&dir).unwrap();
    let sensor = libs.sensors.get(&sensor_id).expect("unknown sensor type");

    let sim = Sim::new(&scenario, &libs, scenario.default_seed).unwrap();
    let terrain = sim.terrain();
    let reds: Vec<(&UnitType, Vec2)> = sim
        .units()
        .iter()
        .filter(|u| u.side == Side::Red)
        .map(|u| (&u.stats, u.pos))
        .collect();

    let cell = terrain.transform().cell_size_m();
    let extent = Vec2::new(
        terrain.width() as f32 * cell,
        terrain.height() as f32 * cell,
    );
    let stride = 250.0f32;

    let mut scored: Vec<(usize, f32, Vec2)> = Vec::new();
    let mut y = stride;
    while y < extent.y {
        let mut x = stride;
        while x < extent.x {
            let pos = Vec2::new(x, y);
            let (seen, total_lambda) = score_site(terrain, sensor, pos, &reds);
            if seen > 0 {
                scored.push((seen, total_lambda, pos));
            }
            x += stride;
        }
        y += stride;
    }

    // Best: most reds seen, then highest total rate.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)));
    println!(
        "Best sites for '{sensor_id}' vs {} red units (of {} candidates seeing >=1):",
        reds.len(),
        scored.len()
    );
    println!("  reds_seen  total_lambda   position");
    for (seen, lambda, pos) in scored.iter().take(12) {
        println!(
            "      {seen:>2}       {lambda:>8.4}     [{:.0}, {:.0}]",
            pos.x, pos.y
        );
    }
}

/// (number of red units with clear LOS in range, sum of detection rates) for a site.
fn score_site(
    terrain: &sim_core::terrain::TerrainGrid,
    sensor: &SensorType,
    pos: Vec2,
    reds: &[(&UnitType, Vec2)],
) -> (usize, f32) {
    let mut seen = 0;
    let mut total = 0.0;
    for (ut, upos) in reds {
        let lambda = detection_rate(terrain, sensor, pos, 0.0, ut, *upos);
        if lambda > 0.0 {
            seen += 1;
            total += lambda;
        }
    }
    (seen, total)
}
