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
