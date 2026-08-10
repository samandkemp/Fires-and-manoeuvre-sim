//! V19-V24 - direct and indirect fires (docs/DESIGN.md §2).
//!
//! Fixtures come from the `validation` crate; the gates reach sim_core through its
//! public API only.

use glam::Vec2;
use rand::SeedableRng;
use sim_core::fires::*;

fn rng(seed: u64) -> sim_core::SimRng {
    sim_core::SimRng::seed_from_u64(seed)
}

#[test]
fn erf_matches_known_values() {
    assert!((erf(0.0)).abs() < 1e-6);
    assert!((erf(1.0) - 0.842_700_8).abs() < 1e-4);
    assert!((erf(-1.0) + 0.842_700_8).abs() < 1e-4);
    assert!((erf(2.0) - 0.995_322_3).abs() < 1e-4);
}

// V19: direct-fire P_hit equals the MC fraction of impacts inside the silhouette.
#[test]
fn v19_direct_hit_probability_monte_carlo() {
    let (disp, range, w, h) = (0.5f32, 1500.0f32, 3.0f32, 2.5f32);
    let analytic = direct_p_hit(disp, range, w, h);
    let sigma = disp * range / 1000.0;

    let n = 40_000;
    let mut hits = 0;
    let mut r = rng(42);
    for _ in 0..n {
        let b = sample_burst(Vec2::ZERO, sigma, &mut r);
        if b.x.abs() <= w / 2.0 && b.y.abs() <= h / 2.0 {
            hits += 1;
        }
    }
    let p_mc = f64::from(hits) / f64::from(n);
    let se = (f64::from(analytic) * (1.0 - f64::from(analytic)) / f64::from(n)).sqrt();
    assert!(
        (p_mc - f64::from(analytic)).abs() < 4.0 * se,
        "P_hit analytic {analytic:.4} vs MC {p_mc:.4} (se {se:.4})"
    );
}

// V20: P_hit monotone - down in range, up in target size.
#[test]
fn v20_direct_hit_monotonicity() {
    let mut last = 1.1;
    for r in [200.0, 500.0, 1000.0, 2000.0, 4000.0] {
        let p = direct_p_hit(0.5, r, 3.0, 2.5);
        assert!(p < last, "P_hit must fall with range");
        last = p;
    }
    let small = direct_p_hit(0.5, 1500.0, 2.0, 1.5);
    let big = direct_p_hit(0.5, 1500.0, 5.0, 3.0);
    assert!(big > small, "P_hit must rise with target size");
}

// V21: sampled bursts have empirical median miss = CEP (Rayleigh median).
#[test]
fn v21_indirect_cep() {
    let cep = 80.0f32;
    let sigma = sigma_from_cep(cep);
    let n = 40_000;
    let mut dists: Vec<f32> = Vec::with_capacity(n);
    let mut r = rng(7);
    for _ in 0..n {
        dists.push(sample_burst(Vec2::ZERO, sigma, &mut r).length());
    }
    dists.sort_by(f32::total_cmp);
    let median = dists[n / 2];
    assert!(
        (median - cep).abs() < 2.0,
        "empirical median miss {median:.1} should equal CEP {cep}"
    );
}

// V22: MC mean Carleton damage over sampled bursts equals the closed form, swept
// over aim offset.
#[test]
fn v22_area_damage_closed_form() {
    let (cep, r_l) = (60.0f32, 40.0f32);
    let sigma = sigma_from_cep(cep);
    let n = 60_000;
    for &d in &[0.0f32, 30.0, 75.0, 150.0] {
        let target = Vec2::new(d, 0.0); // aim at origin, target offset by d
        let analytic = expected_area_damage(d, sigma, r_l);
        let mut sum = 0.0f64;
        let mut r = rng(1000 + d as u64);
        for _ in 0..n {
            let burst = sample_burst(Vec2::ZERO, sigma, &mut r);
            sum += f64::from(carleton_damage(burst.distance(target), r_l));
        }
        let mc = sum / f64::from(n);
        assert!(
            (mc - f64::from(analytic)).abs() < 3e-3,
            "d={d}: E[D] analytic {analytic:.4} vs MC {mc:.4}"
        );
    }
}

// V23: expected damage falls with offset, rises with lethal radius.
#[test]
fn v23_area_damage_monotonicity() {
    let sigma = sigma_from_cep(60.0);
    let mut last = 1.1;
    for d in [0.0, 25.0, 50.0, 100.0, 200.0] {
        let e = expected_area_damage(d, sigma, 40.0);
        assert!(e < last, "E[D] must fall with offset");
        last = e;
    }
    let small_r = expected_area_damage(50.0, sigma, 20.0);
    let big_r = expected_area_damage(50.0, sigma, 60.0);
    assert!(big_r > small_r, "E[D] must rise with lethal radius");
}
