//! The glimpse-rate detection model. Spec: `docs/DESIGN.md` §3. Gates: V14–V18.
//!
//! Only `optical` exists so far. `Modality` and the per-modality signature tables are
//! the seam for acoustic and EO/IR later.

use crate::los;
use crate::terrain::TerrainGrid;
use glam::Vec2;
use std::collections::BTreeMap;

/// The propagation channel a sensor works in. Each modality brings its own terms to
/// the rate model — `Optical` uses LOS + canopy transmittance; future `Acoustic` /
/// `EoIr` variants will add theirs (that's why this is data, not convention).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Line-of-sight visual sensing, attenuated by canopy.
    Optical,
}

/// A sensor type's stat block (`scenarios/sensors.toml`) — all placeholder dials.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorType {
    /// Propagation channel.
    pub modality: Modality,
    /// Height of the sensing element above ground (mast/UAS = big number), metres.
    pub mount_height_m: f32,
    /// Hard detection cutoff, metres.
    pub max_range_m: f32,
    /// Peak detection rate (per second) against signature 1, τ = 1, no concealment,
    /// at point-blank range.
    pub lambda0_per_s: f32,
    /// Range at which the rate falloff halves, metres.
    pub range_half_m: f32,
    /// Falloff steepness `p` in `f(r) = 1 / (1 + (r/range_half)^p)`.
    pub range_exponent: f32,
    /// Field-of-regard width, degrees; omitted = all-round surveillance.
    pub for_width_deg: Option<f32>,
}

/// A unit type's stat block (`scenarios/units.toml`).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitType {
    /// Target height above ground for LOS purposes, metres (also the direct-fire
    /// silhouette height).
    pub height_m: f32,
    /// Direct-fire silhouette width, metres.
    #[serde(default = "default_silhouette_width")]
    pub silhouette_width_m: f32,
    /// Number of sub-elements the unit is made of (e.g. vehicles in a section); attrition
    /// removes them one at a time.
    #[serde(default = "default_element_count")]
    pub element_count: u32,
    /// Movement speed along a route, metres/second (`0` = static).
    #[serde(default)]
    pub speed_m_s: f32,
    /// Per-modality signature in `[0, 1]`-ish (a dial, not a probability): how loud
    /// this unit is in each channel. Keys are modality names (`optical`, …).
    pub signature: BTreeMap<String, f32>,
    /// Weapon type id this unit carries (key into the weapon library), if any.
    #[serde(default)]
    pub weapon: Option<String>,
    /// How much killing **one element** of this unit is worth, for fire allocation
    /// (`docs/DESIGN.md` §10.2). Omit and it is derived from size and weapon threat.
    ///
    /// Per element rather than per unit, so a half-destroyed unit is correctly worth
    /// less than a fresh one. Set it to express doctrine the derived score cannot know —
    /// "kill the radar first" is a `value` an unarmed sensor vehicle would never earn on
    /// its own.
    #[serde(default)]
    pub value: Option<f32>,
    /// Free-form role this asset answers to in a target-priority list
    /// (`docs/DESIGN.md` §13). Optional: the asset class always matches anyway, so this is
    /// only needed to say something finer than "unit".
    #[serde(default)]
    pub role: Option<String>,
}

fn default_silhouette_width() -> f32 {
    3.0
}

fn default_element_count() -> u32 {
    1
}

// Manual `Default` (not derived) so a code-built `UnitType { .., ..Default::default() }`
// gets the same sensible defaults the TOML gives — the derive would zero the silhouette
// width (silently making direct fire never hit) and the element count.
impl Default for UnitType {
    fn default() -> Self {
        Self {
            height_m: 0.0,
            silhouette_width_m: default_silhouette_width(),
            element_count: default_element_count(),
            speed_m_s: 0.0,
            signature: BTreeMap::new(),
            weapon: None,
            value: None,
            role: None,
        }
    }
}

/// The `signature` table key a modality reads. One match arm, in one place, so adding
/// `Acoustic` later cannot leave a signature table silently unread by one caller.
#[must_use]
pub fn modality_key(modality: Modality) -> &'static str {
    match modality {
        Modality::Optical => "optical",
    }
}

/// A per-modality signature lookup: 0 when the table has no entry for the modality — a
/// unit with no `acoustic` key is silent to acoustic sensors.
#[must_use]
pub fn signature_in(signature: &BTreeMap<String, f32>, modality: Modality) -> f32 {
    signature
        .get(modality_key(modality))
        .copied()
        .unwrap_or(0.0)
}

impl UnitType {
    /// The unit's signature in a modality (0 if the table has no entry — a unit with
    /// no `acoustic` entry is silent to acoustic sensors).
    #[must_use]
    pub fn signature_in(&self, modality: Modality) -> f32 {
        signature_in(&self.signature, modality)
    }
}

/// The instantaneous glimpse rate λ (per second) at which `sensor` at `sensor_pos`
/// (facing `facing_deg`, maths convention: 0° = +X/east, CCW) detects a unit of type
/// `unit_type` at `unit_pos`. Zero when hard-blocked, out of range, or outside the
/// field of regard. `docs/DESIGN.md` §3.2.
#[must_use]
pub fn detection_rate(
    terrain: &TerrainGrid,
    sensor: &SensorType,
    sensor_pos: Vec2,
    facing_deg: f32,
    unit_type: &UnitType,
    unit_pos: Vec2,
) -> f32 {
    // A ground unit's concealment is the terrain it stands in (§3.2). Airborne targets
    // take the general form below with `concealment = 0` — they are not in the cell.
    let concealment = concealment_at(terrain, unit_pos);
    detection_rate_against(
        terrain,
        sensor,
        sensor_pos,
        sensor.mount_height_m,
        facing_deg,
        unit_pos,
        unit_type.height_m,
        unit_type.signature_in(sensor.modality),
        concealment,
    )
}

/// The terrain concealment `∈ [0, 1]` at a world position (0 outside the grid).
#[must_use]
pub fn concealment_at(terrain: &TerrainGrid, pos: Vec2) -> f32 {
    match terrain.transform().world_to_cell(pos) {
        Some((ix, iy)) => terrain.concealment()[[iy, ix]],
        None => 0.0,
    }
}

/// The general glimpse rate (`docs/DESIGN.md` §3.2), with every target property passed
/// explicitly rather than read from a [`UnitType`].
///
/// This is the form air assets use: a drone has its own signature and altitude-derived
/// actor height, and contributes `concealment = 0` because an airborne target is not
/// standing in the cell below it (§9.1). `sensor_height_m` is explicit too — a sensor
/// carried by a drone sits at the airframe's height, not at its own `mount_height_m`.
///
/// Composed from [`detection_gate`] and [`rate_given_los`] rather than written out, so a
/// caller that can reuse a line-of-sight result (the sim caches them for endpoints that
/// have not moved) runs the identical arithmetic in the identical order.
// Nine arguments is past clippy's taste threshold, but they are the model's actual
// parameters and bundling them into a struct would only move the noise.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn detection_rate_against(
    terrain: &TerrainGrid,
    sensor: &SensorType,
    sensor_pos: Vec2,
    sensor_height_m: f32,
    facing_deg: f32,
    target_pos: Vec2,
    target_height_m: f32,
    signature: f32,
    concealment: f32,
) -> f32 {
    let Some(r) = detection_gate(
        terrain,
        sensor,
        sensor_pos,
        sensor_height_m,
        facing_deg,
        target_pos,
        target_height_m,
        signature,
    ) else {
        return 0.0;
    };
    let l = los::line_of_sight(
        terrain,
        sensor_pos,
        sensor_height_m,
        target_pos,
        target_height_m,
    );
    rate_given_los(sensor, r, &l, signature, concealment)
}

/// The cheap gates that precede the line-of-sight walk: signature, slant range, and field
/// of regard. Returns the slant range when the sensor could plausibly see the target,
/// `None` when something rules it out.
///
/// Separated because the LOS traversal costs ~77 µs while these cost ~0.1 µs, so the
/// order matters: gate first, walk the grid only if the gates pass.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn detection_gate(
    terrain: &TerrainGrid,
    sensor: &SensorType,
    sensor_pos: Vec2,
    sensor_height_m: f32,
    facing_deg: f32,
    target_pos: Vec2,
    target_height_m: f32,
    signature: f32,
) -> Option<f32> {
    if signature <= 0.0 {
        return None;
    }

    // Slant range, not horizontal (§9.1): overhead is not point blank.
    let r = los::slant_range(
        terrain,
        sensor_pos,
        sensor_height_m,
        target_pos,
        target_height_m,
    );
    if r > sensor.max_range_m {
        return None;
    }

    // Field of regard: bearing to target within ±width/2 of facing.
    if let Some(width) = sensor.for_width_deg {
        let to = target_pos - sensor_pos;
        let bearing = to.y.atan2(to.x).to_degrees();
        let mut off = (bearing - facing_deg) % 360.0;
        if off > 180.0 {
            off -= 360.0;
        } else if off < -180.0 {
            off += 360.0;
        }
        if off.abs() > width * 0.5 {
            return None;
        }
    }
    Some(r)
}

/// The rate itself, given a slant range from [`detection_gate`] and a completed LOS
/// query: `λ₀ · f(r) · signature · τ · (1 − concealment)`, or zero if hard-blocked.
#[must_use]
pub fn rate_given_los(
    sensor: &SensorType,
    slant_range_m: f32,
    los: &los::LosResult,
    signature: f32,
    concealment: f32,
) -> f32 {
    if !los.clear {
        return 0.0;
    }
    let falloff = 1.0 / (1.0 + (slant_range_m / sensor.range_half_m).powf(sensor.range_exponent));
    sensor.lambda0_per_s * falloff * signature * los.transmittance * (1.0 - concealment)
}

/// Probability of at least one detection in a tick of length `dt_s` at rate
/// `lambda_per_s`: `1 − e^{−λ·dt}`. Memoryless, so compounding ticks of any size
/// reproduces `1 − e^{−λt}` exactly (V17).
#[must_use]
pub fn p_detect_tick(lambda_per_s: f32, dt_s: f32) -> f32 {
    1.0 - (-f64::from(lambda_per_s) * f64::from(dt_s)).exp() as f32
}
