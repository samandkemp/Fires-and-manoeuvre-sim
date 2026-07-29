//! Pd validation sweep (the first experiment): confirm the
//! glimpse-rate detection model against its closed form across range. For each range
//! it prints the analytic `P(detect by T) = 1 − e^{−λT}` beside a Monte Carlo estimate
//! that runs the actual per-tick Bernoulli process, so the model *and* the tick
//! machinery are checked together.
//!
//! Run: `cargo run -p experiments --bin pd_sweep [> pd_sweep.csv]`

use glam::Vec2;
use rand::{Rng, SeedableRng};
use sim_core::sensing::{detection_rate, p_detect_tick, Modality, SensorType, UnitType};
use sim_core::terrain::{TerrainParams, TerrainParamsTable, TerrainSource};
use sim_core::SimRng;
use std::collections::BTreeMap;

fn uniform_params() -> TerrainParamsTable {
    let open = TerrainParams {
        feature_height_m: 0.0,
        extinction_per_m: 0.0,
        cover: 0.0,
        concealment: 0.0,
        mobility_cost: 1.0,
    };
    TerrainParamsTable {
        open,
        trees: open,
        urban: open,
    }
}

fn main() {
    let params = uniform_params();
    // Flat, all-open terrain: LOS always clear, τ = 1, no concealment — so the only
    // thing shaping λ is the range falloff, and the closed form is exact.
    let terrain = TerrainSource::Flat { elevation_m: 0.0 }.build(10.0, 500, 40, 1, &params);

    let sensor = SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 5000.0,
        lambda0_per_s: 0.5,
        range_half_m: 1200.0,
        range_exponent: 2.0,
        for_width_deg: None,
    };
    let unit = UnitType {
        height_m: 2.0,
        signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
        ..Default::default()
    };

    let sensor_pos = Vec2::new(50.0, 200.0);
    let exposure_s = 30.0f32;
    let dt = 1.0f32;
    let n = 4000u32;

    println!("range_m,lambda_per_s,p_analytic,p_monte_carlo,abs_err");
    for i in 1..=24 {
        let r = i as f32 * 200.0;
        let target = sensor_pos + Vec2::new(r, 0.0);
        let lambda = detection_rate(&terrain, &sensor, sensor_pos, 0.0, &unit, target);

        let p_analytic = 1.0 - (-lambda * exposure_s).exp();

        // Monte Carlo: run the per-tick process to first detection.
        let p_tick = p_detect_tick(lambda, dt);
        let ticks = (exposure_s / dt) as u32;
        let mut detected = 0u32;
        for seed in 0..n {
            let mut rng = SimRng::seed_from_u64(u64::from(seed));
            for _ in 0..ticks {
                if rng.random::<f32>() < p_tick {
                    detected += 1;
                    break;
                }
            }
        }
        let p_mc = f64::from(detected) / f64::from(n);
        println!(
            "{r:.0},{lambda:.6},{p_analytic:.4},{p_mc:.4},{:.4}",
            (p_mc - f64::from(p_analytic)).abs()
        );
    }
    eprintln!("(flat open terrain, exposure {exposure_s:.0} s, {n} MC trials/range)");
}
