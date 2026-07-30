//! Least-risk pathing demo: build the default scenario, form a risk raster from Red's
//! observation coverage (how detectable a mover is at each cell), then plan a Blue move
//! at increasing caution. Higher `risk_weight` trades a longer route for less exposure —
//! "move without being seen" made navigable.
//!
//! Run: `cargo run -p experiments --bin risk_path`

use glam::Vec2;
use ndarray::Array2;
use sim_core::movement::{least_risk_path, path_risk};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::detection_rate;
use sim_core::sim::{Side, Sim};
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let scenario = Scenario::load(&dir.join("default.toml")).unwrap();
    let libs = Libraries::load_dir(&dir).unwrap();
    let sim = Sim::new(&scenario, &libs, scenario.default_seed).unwrap();
    let terrain = sim.terrain();

    // Risk raster: max Red-sensor detection rate against a reference mover per cell.
    let mover = &libs.units["afv"];
    let red_sensors: Vec<_> = sim
        .sensors()
        .iter()
        .filter(|s| s.side == Side::Red)
        .collect();
    let t0 = std::time::Instant::now();
    let mut risk = Array2::<f32>::zeros((terrain.height(), terrain.width()));
    // Parallel over cells (each writes its own slot).
    ndarray::Zip::indexed(&mut risk).par_for_each(|(iy, ix), v| {
        let cell = terrain.transform().cell_center(ix, iy);
        let mut r = 0.0f32;
        for s in &red_sensors {
            r = r.max(detection_rate(
                terrain,
                &s.stats,
                s.pos,
                s.facing_deg,
                mover,
                cell,
            ));
        }
        *v = r;
    });
    let max_r = risk.iter().copied().fold(0.0f32, f32::max);
    if max_r > 0.0 {
        risk.mapv_inplace(|v| v / max_r); // normalise to [0, 1]
    }
    eprintln!(
        "risk raster ({} red sensors) built in {:?}",
        red_sensors.len(),
        t0.elapsed()
    );

    let cell = terrain.transform().cell_size_m();
    let to_cell = |p: Vec2| ((p.x / cell).floor() as usize, (p.y / cell).floor() as usize);
    let start = to_cell(Vec2::new(5200.0, 2500.0)); // south of the Red observer
    let goal = to_cell(Vec2::new(7400.0, 7100.0)); // the Red AFV — direct route crosses coverage

    println!("Planning move ({start:?} → {goal:?}) at rising caution:");
    println!("  risk_weight   path_cells   total_cost   risk_exposure");
    for &wgt in &[0.0f32, 50.0, 200.0, 800.0] {
        match least_risk_path(terrain, &risk, start, goal, wgt) {
            Some(p) => println!(
                "  {wgt:>9.0}   {:>9}   {:>10.0}   {:>10.2}",
                p.cells.len(),
                p.cost,
                path_risk(&p, &risk)
            ),
            None => println!("  {wgt:>9.0}   (unreachable)"),
        }
    }
    println!("(higher caution → longer path, lower cumulative exposure)");
}
