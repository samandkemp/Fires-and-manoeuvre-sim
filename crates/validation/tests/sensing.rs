//! V14-V18 - the glimpse-rate detection model (docs/DESIGN.md §3).
//!
//! Fixtures come from the `validation` crate; the gates reach sim_core through its
//! public API only.

use glam::Vec2;
use ndarray::Array2;
use sim_core::sensing::*;
use sim_core::terrain::{TerrainGrid, TerrainType};
use std::collections::BTreeMap;
use validation::{flat, params};

fn sensor() -> SensorType {
    SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 4000.0,
        lambda0_per_s: 0.5,
        range_half_m: 1200.0,
        range_exponent: 2.0,
        for_width_deg: None,
    }
}

fn unit() -> UnitType {
    UnitType {
        height_m: 2.0,
        signature: BTreeMap::from([("optical".into(), 0.8)]),
        ..Default::default()
    }
}

// V16: rate structure - monotonicity, gating, and linear scaling.
#[test]
fn v16_rate_structure() {
    let g = flat(64, 64);
    let s = sensor();
    let u = unit();
    let sp = Vec2::new(50.0, 320.0);

    // Monotone non-increasing in range.
    let mut last = f32::INFINITY;
    for x in [100.0, 300.0, 600.0, 1200.0, 2400.0, 3900.0] {
        let rate = detection_rate(&g, &s, sp, 0.0, &u, Vec2::new(x, 320.0));
        assert!(rate > 0.0 && rate <= last, "rate must fall with range");
        last = rate;
    }

    // Beyond max range: exactly zero.
    assert_eq!(
        detection_rate(&g, &s, sp, 0.0, &u, Vec2::new(4500.0, 320.0)),
        0.0
    );

    // Outside the field of regard: exactly zero; inside: positive.
    let mut narrow = sensor();
    narrow.for_width_deg = Some(60.0);
    assert!(detection_rate(&g, &narrow, sp, 0.0, &u, Vec2::new(500.0, 320.0)) > 0.0);
    assert_eq!(
        detection_rate(&g, &narrow, sp, 90.0, &u, Vec2::new(500.0, 320.0)),
        0.0,
        "target due east must be invisible to a north-facing 60° sensor"
    );

    // Linear in signature.
    let mut quiet = unit();
    quiet.signature.insert("optical".into(), 0.4);
    let loud_rate = detection_rate(&g, &s, sp, 0.0, &u, Vec2::new(600.0, 320.0));
    let quiet_rate = detection_rate(&g, &s, sp, 0.0, &quiet, Vec2::new(600.0, 320.0));
    assert!(
        (loud_rate / quiet_rate - 2.0).abs() < 1e-4,
        "rate ∝ signature"
    );

    // Concealment at the target cell reduces the rate (Trees concealment 0.6).
    let tt = Array2::from_shape_fn((64, 64), |(iy, ix)| {
        if ix == 60 && iy == 32 {
            TerrainType::Trees
        } else {
            TerrainType::Open
        }
    });
    let g2 = TerrainGrid::from_layers(10.0, Array2::zeros((64, 64)), tt, &params());
    // Target *in* the woods cell (its own canopy doesn't block LOS - endpoint
    // exclusion - but its concealment applies).
    let open_rate = detection_rate(&g, &s, sp, 0.0, &u, Vec2::new(605.0, 325.0));
    let hidden_rate = detection_rate(&g2, &s, sp, 0.0, &u, Vec2::new(605.0, 325.0));
    assert!(
        (hidden_rate / open_rate - 0.4).abs() < 1e-3,
        "concealment 0.6 must scale the rate by 0.4 (got ratio {})",
        hidden_rate / open_rate
    );

    // A unit with no entry for the modality is undetectable.
    let silent = UnitType {
        height_m: 2.0,
        signature: BTreeMap::new(),
        ..Default::default()
    };
    assert_eq!(
        detection_rate(&g, &s, sp, 0.0, &silent, Vec2::new(500.0, 320.0)),
        0.0
    );
}

// V17: compounding per-tick survival reproduces e^{−λt} for any tick size.
#[test]
fn v17_tick_size_invariance() {
    let lambda = 0.23f32;
    let t_total = 12.0f32;
    let exact = (-f64::from(lambda) * f64::from(t_total)).exp();
    for dt in [0.25f32, 0.5, 1.0, 2.0] {
        let ticks = (t_total / dt).round() as u32;
        let mut survival = 1.0f64;
        for _ in 0..ticks {
            survival *= 1.0 - f64::from(p_detect_tick(lambda, dt));
        }
        assert!(
            (survival - exact).abs() < 1e-5,
            "dt={dt}: compounded survival {survival} vs exact {exact}"
        );
    }
}
