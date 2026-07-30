//! V40-V43 - electronic warfare and the POMDP belief layer (docs/DESIGN.md §8).
//!
//! Fixtures come from the `validation` crate; the gates reach sim_core through its
//! public API only.

use glam::Vec2;
use sim_core::ew::*;

// V40: EW-off is the identity — no jammers ⇒ factor exactly 1.
#[test]
fn v40_no_jammers_is_identity() {
    assert_eq!(jamming_factor(Vec2::new(500.0, 500.0), &[]), 1.0);
}

#[test]
fn jamming_is_monotone_and_bounded() {
    let j = Jammer {
        pos: Vec2::ZERO,
        power: 0.8,
        radius_m: 1000.0,
    };
    let js = [j];
    // At the centre: strongest degradation = 1 − power.
    assert!((jamming_factor(Vec2::ZERO, &js) - 0.2).abs() < 1e-6);
    // Monotone increasing back to 1 with distance.
    let mut last = 0.0;
    for d in [0.0, 200.0, 500.0, 800.0, 999.0] {
        let f = jamming_factor(Vec2::new(d, 0.0), &js);
        assert!(f >= last - 1e-6, "factor must rise with distance");
        last = f;
    }
    // Beyond the radius: no effect.
    assert_eq!(jamming_factor(Vec2::new(1200.0, 0.0), &js), 1.0);
    // Stronger jammer degrades more.
    let strong = [Jammer {
        pos: Vec2::ZERO,
        power: 1.0,
        radius_m: 1000.0,
    }];
    assert!(
        jamming_factor(Vec2::new(300.0, 0.0), &strong) < jamming_factor(Vec2::new(300.0, 0.0), &js)
    );
}

#[test]
fn jammers_compose_multiplicatively() {
    let a = Jammer {
        pos: Vec2::ZERO,
        power: 0.5,
        radius_m: 1000.0,
    };
    let b = Jammer {
        pos: Vec2::new(100.0, 0.0),
        power: 0.5,
        radius_m: 1000.0,
    };
    let p = Vec2::new(50.0, 0.0);
    let expected = jamming_factor(p, &[a]) * jamming_factor(p, &[b]);
    assert!((jamming_factor(p, &[a, b]) - expected).abs() < 1e-6);
}

use ndarray::Array2;
use sim_core::pomdp::*;
use sim_core::sensing::{Modality, SensorType, UnitType};
use std::collections::BTreeMap;
use validation::flat;

// ---- V41-V43: the POMDP belief layer -------------------------------------

// V41: the Tiger problem — Bayes updates reproduce the exact posteriors.
#[test]
fn v41_tiger_problem() {
    // States: tiger-left (0), tiger-right (1). "Listen" hears the correct side with
    // accuracy 0.85. Observation "hear-left" likelihood = (0.85, 0.15).
    let hear_left = [0.85f32, 0.15];
    let prior = [0.5f32, 0.5];

    let after_one = bayes_update(&prior, &hear_left);
    assert!(
        (after_one[0] - 0.85).abs() < 1e-5,
        "one hear-left → 0.85 left"
    );

    let after_two = bayes_update(&after_one, &hear_left);
    // 0.85² / (0.85² + 0.15²) = 0.7225 / 0.745 = 0.96979…
    assert!(
        (after_two[0] - 0.969_79).abs() < 1e-4,
        "two hear-left → ~0.9698 left"
    );

    // A contradicting "hear-right" observation pulls the belief back toward even.
    let hear_right = [0.15f32, 0.85];
    let mixed = bayes_update(&after_two, &hear_right);
    assert!(
        (mixed[0] - 0.85).abs() < 1e-4,
        "hear-right after two hear-left → back to 0.85"
    );
}

fn optical() -> SensorType {
    SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 1200.0,
        lambda0_per_s: 1.0,
        range_half_m: 800.0,
        range_exponent: 2.0,
        for_width_deg: None,
    }
}

fn target() -> UnitType {
    UnitType {
        height_m: 2.0,
        signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
        ..Default::default()
    }
}

// V42: belief is a proper distribution and a peaked observation concentrates it.
#[test]
fn v42_belief_is_proper_and_concentrates() {
    let (w, h) = (30, 30);
    let mut belief = SpatialBelief::uniform(w, h);
    assert!((belief.belief().sum() - 1.0).abs() < 1e-4);

    // A sharp likelihood peaked at cell (20, 8) — a detection there.
    let mut like = Array2::from_elem((h, w), 0.001f32);
    like[[8, 20]] = 1.0;
    belief.update(&like);

    assert!(
        (belief.belief().sum() - 1.0).abs() < 1e-4,
        "belief must stay normalised"
    );
    assert!(belief.belief().iter().all(|&p| p >= 0.0));
    assert_eq!(
        belief.most_likely_cell(),
        (20, 8),
        "belief should concentrate on the detection"
    );
    assert!(
        belief.entropy() < SpatialBelief::uniform(w, h).entropy(),
        "a detection reduces entropy"
    );
}

// V43: negative information — repeatedly *not* detecting from a sensor shifts belief
// out of its coverage into dead ground; the motion model raises uncertainty.
#[test]
fn v43_negative_information_and_diffusion() {
    let (w, h) = (60, 20);
    let terrain = flat(w, h);
    let sensor = optical();
    let target = target();
    // Sensor at the west end; its ~1.2 km range covers the western cells only.
    let sensor_pos = Vec2::new(50.0, 100.0);
    let like = no_detection_likelihood(&terrain, &sensor, sensor_pos, 0.0, &target, &[], 1.0);

    let mut belief = SpatialBelief::uniform(w, h);
    for _ in 0..15 {
        belief.update(&like);
    }
    // "West" = within the sensor's reach (ix < 20 ≈ 200 m... actually within 1200 m,
    // ix < 120 — but map is 600 m wide, so split at mid). Compare covered vs far.
    let west = belief.mass_where(|ix, _| ix < w / 3);
    let east = belief.mass_where(|ix, _| ix >= 2 * w / 3);
    assert!(
        east > west * 2.0,
        "belief must flee the watched west ({west}) toward the east ({east})"
    );

    // The motion model spreads mass back out (raises entropy).
    let before = belief.entropy();
    belief.predict(0.3);
    assert!(
        belief.entropy() > before,
        "diffusion must raise uncertainty"
    );
    assert!(
        (belief.belief().sum() - 1.0).abs() < 1e-3,
        "still normalised after predict"
    );
}
