//! Direct-fire hit probability, indirect-fire dispersion and area effect.
//! Spec: `docs/DESIGN.md` §2. Gates: V19-V24.
//!
//! Pure functions only - `sim` drives them into a battle.

// Constants are written at full mathematical precision (A&S 7.1.26, √(2 ln 2)) so they
// stay recognisable, even though f32 rounds them.
#![allow(clippy::excessive_precision)]

use glam::Vec2;
use rand::Rng;
use rand_distr::{Distribution, Normal};

/// √(2·ln 2), where `CEP = σ · CEP_FACTOR` for a circular Gaussian.
pub const CEP_FACTOR: f32 = 1.177_410_0;

/// How a weapon delivers effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponClass {
    /// Flat trajectory, needs LOS, hits against a silhouette.
    Direct,
    /// Ballistic arc, dispersion and area effect around an aim point.
    Indirect,
}

/// Weapon stat block (`scenarios/weapons.toml`). Placeholder dials.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponType {
    /// Direct or indirect.
    pub class: WeaponClass,
    /// Rounds per minute; the sim converts this to a per-epoch count.
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
    /// Direct: σ multiplier against a moving target. Unused so far.
    #[serde(default = "one")]
    pub moving_target_penalty: f32,
    /// Indirect: circular error probable, metres.
    #[serde(default)]
    pub cep_m: f32,
    /// Indirect: Carleton lethal-radius scale `R_L`, metres.
    #[serde(default)]
    pub lethal_radius_m: f32,
    /// Can this engage air targets (§9.6)? Unused: target selection only iterates the
    /// unit list, so a ground weapon can't pick a drone anyway. Here for a future
    /// dual-role autocannon.
    #[serde(default)]
    pub engages_air: bool,
    /// Does this munition home on a **transmitting** emitter (`docs/DESIGN.md` §12.3)?
    ///
    /// An anti-radiation missile rides the radar's own signal down, so its accuracy is
    /// bought with the target's emissions. Switching the radar off should therefore be a
    /// counter - which is the trade this flag exists to pose.
    #[serde(default)]
    pub anti_radiation: bool,
    /// Circular error probable against a target that is **not** transmitting, metres.
    ///
    /// Only read when `anti_radiation` is set. The munition still arrives - it flies to
    /// where the emitter was last known to be - but with nothing to home on it lands with
    /// this dispersion instead of `cep_m`. Defaults to `cep_m`, so declaring
    /// `anti_radiation` alone changes nothing until the degradation is stated.
    ///
    /// A dial rather than a rule, because "an ARM cannot engage a silent radar at all" is
    /// just this with the value set very large - reachable, but as a scenario's choice
    /// rather than the model's assumption.
    #[serde(default)]
    pub silent_cep_m: Option<f32>,
}

impl WeaponType {
    /// The dispersion this munition lands with against a target that is or is not
    /// currently transmitting (`docs/DESIGN.md` §12.3).
    ///
    /// For everything that is not an ARM this is `cep_m` regardless - the emitter state is
    /// simply not part of a dumb shell's accuracy - which is what makes the whole mechanism
    /// an exact identity for every existing weapon.
    #[must_use]
    pub fn cep_against(&self, emitting: bool) -> f32 {
        if self.anti_radiation && !emitting {
            self.silent_cep_m.unwrap_or(self.cep_m)
        } else {
            self.cep_m
        }
    }
}

fn one() -> f32 {
    1.0
}

// Hand-written Default, not derived: deriving would zero `p_kill_given_hit` and
// `moving_target_penalty`, quietly making every code-built weapon harmless. This matches
// what the TOML defaults give.
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
            anti_radiation: false,
            silent_cep_m: None,
        }
    }
}

/// Error function (A&S 7.1.26, max error ~1.5e-7). std has no `erf`.
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

/// Single-shot hit probability against a `width × height` silhouette at `range_m`.
///
/// Impacts scatter as an isotropic 2-D Gaussian about the target centre. Deflection and
/// elevation are independent, hence the product: `erf(W/2σ√2) · erf(H/2σ√2)` (V19).
#[must_use]
pub fn direct_p_hit(dispersion_mrad: f32, range_m: f32, width_m: f32, height_m: f32) -> f32 {
    let sigma = (dispersion_mrad * range_m / 1000.0).max(1e-6);
    let k = 1.0 / (2.0 * sigma * std::f32::consts::SQRT_2);
    erf(width_m * k) * erf(height_m * k)
}

/// The σ implied by a circular error probable, metres.
#[must_use]
pub fn sigma_from_cep(cep_m: f32) -> f32 {
    cep_m / CEP_FACTOR
}

/// Carleton kernel: `exp(−ρ²/2R_L²)`, the chance of incapacitation `rho_m` from a burst.
#[must_use]
pub fn carleton_damage(rho_m: f32, r_l_m: f32) -> f32 {
    (-rho_m * rho_m / (2.0 * r_l_m * r_l_m)).exp()
}

/// Expected Carleton damage from one round aimed `offset_m` off a point target,
/// averaged over the burst dispersion. Gaussian convolution, so it has a closed form:
/// `R_L²/(σ²+R_L²) · exp(−d²/2(σ²+R_L²))` (V22).
#[must_use]
pub fn expected_area_damage(offset_m: f32, sigma_m: f32, r_l_m: f32) -> f32 {
    let s2 = sigma_m * sigma_m + r_l_m * r_l_m;
    (r_l_m * r_l_m / s2) * (-offset_m * offset_m / (2.0 * s2)).exp()
}

/// Draw a burst point about `aim` with per-axis dispersion `sigma_m`.
#[must_use]
pub fn sample_burst(aim: Vec2, sigma_m: f32, rng: &mut impl Rng) -> Vec2 {
    let n = Normal::new(0.0f32, sigma_m.max(1e-6)).expect("sigma is finite and positive");
    aim + Vec2::new(n.sample(rng), n.sample(rng))
}
