//! Electronic warfare: a modifier on the sensing channel (`docs/DESIGN.md` §8).
//! A jammer protects its own side's units by degrading the enemy's detection of them —
//! a multiplicative factor on the glimpse rate λ. With no jammers the factor is exactly
//! 1, so EW-off reduces the sensing model bit-for-bit to Phase 2 (validated V40).

use glam::Vec2;

/// A placed jammer: a protective bubble that degrades detection of nearby friendly units.
#[derive(Clone, Copy, Debug)]
pub struct Jammer {
    /// Centre position, world metres.
    pub pos: Vec2,
    /// Peak degradation at the centre, `[0, 1]` (1 = fully blinds the sensor there).
    pub power: f32,
    /// Effect radius, metres (linear falloff to 0 at the edge).
    pub radius_m: f32,
}

/// The multiplicative detection-degradation factor at `target` from `jammers` (which
/// should be the jammers on the target's own side). `1` = no jamming; `→ 0` = blinded.
///
/// Each jammer contributes `g = 1 − power·(1 − d/radius)` inside its radius (so `1−power`
/// at the centre, `1` at the edge), and factors compose multiplicatively.
#[must_use]
pub fn jamming_factor(target: Vec2, jammers: &[Jammer]) -> f32 {
    let mut factor = 1.0f32;
    for j in jammers {
        let d = target.distance(j.pos);
        if d < j.radius_m {
            let g = 1.0 - j.power * (1.0 - d / j.radius_m);
            factor *= g.clamp(0.0, 1.0);
        }
    }
    factor
}

#[cfg(test)]
mod tests {
    use super::*;

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
            jamming_factor(Vec2::new(300.0, 0.0), &strong)
                < jamming_factor(Vec2::new(300.0, 0.0), &js)
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
}
