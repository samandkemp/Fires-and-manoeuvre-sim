//! Performance baseline harness (P0). Times the hot paths so optimisation is guided by
//! measurement, not guesswork (a project guardrail). Plain `std::time` - no bench crate.
//!
//! Run: `cargo run -p experiments --release --bin bench`
//!      `cargo run -p experiments --bin bench   # dev profile (what the app uses)`

use glam::Vec2;
use sim_core::los;
use sim_core::scenario::{load_terrain_params, Scenario};
use std::time::Instant;

fn main() {
    let profile = if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    };
    println!("=== bench ({profile} profile) ===");

    let scn = Scenario::from_toml_str(
        r#"
        name = "bench"
        [terrain]
        cell_size_m = 10.0
        width_cells = 1000
        height_cells = 1000
        [terrain.source.hills]
        count = 24
        max_height_m = 120.0
        base_radius_m = 600.0
        woods_fraction = 0.28
        urban_blocks = 4
    "#,
    )
    .unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let params = load_terrain_params(&dir.join("terrain_types.toml")).unwrap();

    let t = Instant::now();
    let terrain = scn.build_terrain(&params, 1);
    println!("terrain gen 1000x1000 hills : {:>8.1?}", t.elapsed());

    // LOS throughput over random long rays.
    let n = 200_000u64;
    let mut s = 0x1234_5678u64;
    let mut rnd = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as f32 / (u32::MAX >> 1) as f32
    };
    let mut acc = 0.0f32;
    let t = Instant::now();
    for _ in 0..n {
        let a = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
        let b = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
        acc += los::line_of_sight(&terrain, a, 2.0, b, 2.0).transmittance;
    }
    let el = t.elapsed();
    println!(
        "LOS {n} random rays        : {:>8.1?}  = {:>6.2} us/query   (acc {acc:.0})",
        el,
        el.as_secs_f64() * 1e6 / n as f64
    );

    // Viewshed: coverage raster over the whole 1M-cell map.
    for range in [1500.0f32, 3000.0, 12000.0] {
        let t = Instant::now();
        let vs = los::viewshed(&terrain, Vec2::new(5000.0, 5000.0), 2.0, 2.0, range);
        let seen = vs.iter().filter(|&&v| v > 0.0).count();
        println!(
            "viewshed 1M cells, {range:>5.0} m   : {:>8.1?}  (seen {seen})",
            t.elapsed()
        );
    }

    // Slant range (docs/DESIGN.md §9.1): the cost of the convention, and how far it
    // actually moves the answer on relief - the number behind "no re-baseline needed".
    let mut acc = 0.0f32;
    let t = Instant::now();
    for _ in 0..n {
        let a = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
        let b = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
        acc += los::slant_range(&terrain, a, 2.0, b, 2.0);
    }
    let el = t.elapsed();
    println!(
        "slant_range {n} queries    : {:>8.1?}  = {:>6.3} us/query   (acc {acc:.0})",
        el,
        el.as_secs_f64() * 1e6 / n as f64
    );

    // How much does slant differ from horizontal on this map? Ground-to-ground first
    // (the existing gates), then against an airborne endpoint (what Phase 9 needed).
    let report = |label: &str, h_b: f32, samples: u32, rnd: &mut dyn FnMut() -> f32| {
        let (mut worst, mut worst_rel, mut sum_rel) = (0.0f32, 0.0f32, 0.0f64);
        for _ in 0..samples {
            let a = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
            let b = Vec2::new(rnd() * 9900.0, rnd() * 9900.0);
            let horizontal = a.distance(b);
            if horizontal < 1.0 {
                continue;
            }
            let slant = los::slant_range(&terrain, a, 2.0, b, h_b);
            let delta = slant - horizontal;
            let rel = delta / horizontal;
            worst = worst.max(delta);
            worst_rel = worst_rel.max(rel);
            sum_rel += f64::from(rel);
        }
        println!(
            "  {label:<28}: mean +{:.4}%  worst +{:.2}% ({:.0} m)",
            sum_rel / f64::from(samples) * 100.0,
            worst_rel * 100.0,
            worst
        );
    };
    println!("slant vs horizontal range on 120 m relief:");
    report("ground-ground (h=2 m)", 2.0, 20_000, &mut rnd);
    report("ground-air    (h=400 m)", 400.0, 20_000, &mut rnd);

    // The simulation tick: what every batch run and every app frame pays, and what the
    // rasters above don't cover.
    //
    // Treat the tick figure as a sanity check, not an optimisation target - it is
    // sub-millisecond and swings 2-3x run to run on a busy machine. `build` is the number
    // that matters, paid on every scenario load.
    println!(
        "
simulation tick (shipped scenarios):"
    );
    let libs = sim_core::scenario::Libraries::load_dir(&dir).unwrap();
    for name in ["default", "air_raid", "mountain_pass"] {
        let Ok(scn) = sim_core::scenario::Scenario::load(&dir.join(format!("{name}.toml"))) else {
            continue;
        };
        let t = Instant::now();
        let Ok(mut sim) = sim_core::sim::Sim::new(&scn, &libs, scn.default_seed) else {
            continue;
        };
        let build = t.elapsed();

        // Reset cost matters too: it is what a batch run pays per trial.
        let t = Instant::now();
        sim.reset_to_scenario(&scn, &libs, 1).unwrap();
        let reset = t.elapsed();

        let ticks = 2000u32;
        let t = Instant::now();
        for _ in 0..ticks {
            sim.step_one();
        }
        let el = t.elapsed();
        // The LOS memo is what took the tick from ~100 us to ~10 us: a sensor re-testing
        // a target that has not moved re-walks the same terrain for the same answer. A
        // low hit rate is the signal to look at (everything is moving, or the cache is
        // being invalidated), not a high one.
        let (hits, misses) = sim.los_cache_stats();
        let rate = if hits + misses == 0 {
            0.0
        } else {
            hits as f64 * 100.0 / (hits + misses) as f64
        };
        println!(
            "  {name:<14} build {:>7.1?}  reset {:>7.1?}  tick {:>6.1} us  (LOS memo {rate:.0}% of {} lookups)",
            build,
            reset,
            el.as_secs_f64() * 1e6 / f64::from(ticks),
            hits + misses
        );
    }
}
