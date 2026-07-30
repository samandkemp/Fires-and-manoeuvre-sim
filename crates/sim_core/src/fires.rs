//! Fires: direct-fire hit probability and indirect-fire dispersion + area effect.
//! Specified in `docs/DESIGN.md` §2; validated by V19–V24. Pure model functions here;
//! the sim loop (`sim.rs`) drives them into a battle.

// The erf and CEP constants below are the canonical full-precision mathematical values
// (Abramowitz & Stegun 7.1.26; √(2 ln 2)); keep them recognisable and f64-ready even
// though f32 rounds them.
#![allow(clippy::excessive_precision)]

use glam::Vec2;
use rand::Rng;
use rand_distr::{Distribution, Normal};

/// √(2·ln 2): the circular-Gaussian CEP factor, `CEP = σ · CEP_FACTOR`.
pub const CEP_FACTOR: f32 = 1.177_410_0;

/// What a weapon is and how it delivers effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponClass {
    /// Flat-trajectory, LOS-gated, hit against a target silhouette.
    Direct,
    /// Ballistic, dispersion + area effect around an aim point.
    Indirect,
}

/// A weapon type's stat block (`scenarios/weapons.toml`) — placeholder dials.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct WeaponType {
    /// Direct or indirect.
    pub class: WeaponClass,
    /// Rounds per minute (converted to per-tick expectations by the sim).
    pub rof_rounds_per_min: f32,
    /// Maximum engagement range, metres.
    pub max_range_m: f32,
    /// Minimum range, metres (indirect only; default 0).
    #[serde(default)]
    pub min_range_m: f32,
    /// Direct: 1σ angular aiming error, milliradians.
    #[serde(default)]
    pub dispersion_mrad: f32,
    /// Direct: probability of a kill given the round strikes the silhouette.
    #[serde(default = "one")]
    pub p_kill_given_hit: f32,
    /// Direct: σ-inflation factor against a moving target (dormant until movement).
    #[serde(default = "one")]
    pub moving_target_penalty: f32,
    /// Indirect: circular error probable, metres.
    #[serde(default)]
    pub cep_m: f32,
    /// Indirect: Carleton lethal-radius scale `R_L`, metres.
    #[serde(default)]
    pub lethal_radius_m: f32,
    /// May this weapon engage air targets (`docs/DESIGN.md` §9.6)? Dormant seam: air
    /// defence is its own class, and ground target selection iterates only the unit
    /// list, so a ground weapon structurally cannot pick a drone today. This is the
    /// opt-in a future dual-role autocannon would flip.
    #[serde(default)]
    pub engages_air: bool,
}

fn one() -> f32 {
    1.0
}

// Manual `Default` (not derived), for the same reason `UnitType` has one: the derive
// would zero `p_kill_given_hit` and `moving_target_penalty`, silently making any
// code-built weapon incapable of killing anything. This keeps a
// `WeaponType { .., ..Default::default() }` literal agreeing with what the TOML defaults
// would have given.
impl Default for WeaponType {
    fn default() -> Self {
        Self {
            class: WeaponClass::Direct,
            rof_rounds_per_min: 0.0,
            max_range_m: 0.0,
            min_range_m: 0.0,
            dispersion_mrad: 0.0,
            p_kill_given_hit: one(),
            moving_target_penalty: one(),
            cep_m: 0.0,
            lethal_radius_m: 0.0,
            engages_air: false,
        }
    }
}

/// Error function, Abramowitz & Stegun 7.1.26 (max abs error ~1.5e-7). std has no `erf`.
#[must_use]
pub fn erf(x: f32) -> f32 {
    let sign = x.signum();
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    sign * (1.0 - poly * (-x * x).exp())
}

/// Direct-fire single-shot hit probability against a `width × height` metre silhouette
/// at `range_m`, with 1σ dispersion `dispersion_mrad`. The impact scatters as an
/// isotropic 2-D Gaussian about the target centre; deflection and elevation hits are
/// independent, so `P_hit = erf(W / 2σ√2) · erf(H / 2σ√2)` (V19).
#[must_use]
pub fn direct_p_hit(dispersion_mrad: f32, range_m: f32, width_m: f32, height_m: f32) -> f32 {
    let sigma = (dispersion_mrad * range_m / 1000.0).max(1e-6);
    let k = 1.0 / (2.0 * sigma * std::f32::consts::SQRT_2);
    erf(width_m * k) * erf(height_m * k)
}

/// The dispersion σ (metres) implied by a circular error probable.
#[must_use]
pub fn sigma_from_cep(cep_m: f32) -> f32 {
    cep_m / CEP_FACTOR
}

/// Carleton damage kernel: probability of incapacitation at distance `rho_m` from a
/// burst with lethal-radius scale `r_l_m`, `exp(−ρ² / 2R_L²)`.
#[must_use]
pub fn carleton_damage(rho_m: f32, r_l_m: f32) -> f32 {
    (-rho_m * rho_m / (2.0 * r_l_m * r_l_m)).exp()
}

/// Expected Carleton damage to a point target from a single round aimed with offset
/// `offset_m` from it, marginalised over a σ-dispersion Gaussian burst — the closed-form
/// Gaussian convolution `R_L²/(σ²+R_L²) · exp(−d²/2(σ²+R_L²))` (V22).
#[must_use]
pub fn expected_area_damage(offset_m: f32, sigma_m: f32, r_l_m: f32) -> f32 {
    let s2 = sigma_m * sigma_m + r_l_m * r_l_m;
    (r_l_m * r_l_m / s2) * (-offset_m * offset_m / (2.0 * s2)).exp()
}

/// Sample a dispersed burst point about `aim` with per-axis dispersion `sigma_m`.
#[must_use]
pub fn sample_burst(aim: Vec2, sigma_m: f32, rng: &mut impl Rng) -> Vec2 {
    let n = Normal::new(0.0f32, sigma_m.max(1e-6)).expect("sigma is finite and positive");
    aim + Vec2::new(n.sample(rng), n.sample(rng))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng(seed: u64) -> crate::SimRng {
        crate::SimRng::seed_from_u64(seed)
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

    // V20: P_hit monotone — down in range, up in target size.
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
}
