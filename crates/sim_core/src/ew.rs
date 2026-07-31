//! Jamming as a modifier on the sensing channel. Spec: `docs/DESIGN.md` §8. Gate: V40.
//!
//! A jammer protects its own side by degrading enemy detection of it: a multiplicative
//! factor on the glimpse rate λ. With no jammers the factor is exactly 1, so EW-off is
//! bit-for-bit identical to the plain sensing model.

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
