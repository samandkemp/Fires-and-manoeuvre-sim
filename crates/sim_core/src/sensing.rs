//! Sensing & detection: the glimpse-rate probability-of-detection model.
//! Specified in `docs/DESIGN.md` §3; validated by V14–V18.
//!
//! One generic LOS (`optical`) modality for now; the `Modality` tag and per-modality
//! signature tables are the seam acoustic and EO/IR sensing slot into later.

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
    if signature <= 0.0 {
        return 0.0;
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
        return 0.0;
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
            return 0.0;
        }
    }

    let l = los::line_of_sight(
        terrain,
        sensor_pos,
        sensor_height_m,
        target_pos,
        target_height_m,
    );
    if !l.clear {
        return 0.0;
    }

    let falloff = 1.0 / (1.0 + (r / sensor.range_half_m).powf(sensor.range_exponent));
    sensor.lambda0_per_s * falloff * signature * l.transmittance * (1.0 - concealment)
}

/// Probability of at least one detection in a tick of length `dt_s` at rate
/// `lambda_per_s`: `1 − e^{−λ·dt}`. Memoryless, so compounding ticks of any size
/// reproduces `1 − e^{−λt}` exactly (V17).
#[must_use]
pub fn p_detect_tick(lambda_per_s: f32, dt_s: f32) -> f32 {
    1.0 - (-f64::from(lambda_per_s) * f64::from(dt_s)).exp() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{TerrainGrid, TerrainParams, TerrainParamsTable, TerrainType};
    use ndarray::Array2;

    fn params() -> TerrainParamsTable {
        let mk = |fh, k, cov, con, mob| TerrainParams {
            feature_height_m: fh,
            extinction_per_m: k,
            cover: cov,
            concealment: con,
            mobility_cost: mob,
        };
        TerrainParamsTable {
            open: mk(0.0, 0.0, 0.0, 0.0, 1.0),
            trees: mk(12.0, 0.08, 0.3, 0.6, 1.8),
            urban: mk(8.0, 0.0, 0.7, 0.5, 1.5),
        }
    }

    fn flat(w: usize, h: usize) -> TerrainGrid {
        TerrainGrid::from_layers(
            10.0,
            Array2::zeros((h, w)),
            Array2::from_elem((h, w), TerrainType::Open),
            &params(),
        )
    }

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

    // V16: rate structure — monotonicity, gating, and linear scaling.
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
        // Target *in* the woods cell (its own canopy doesn't block LOS — endpoint
        // exclusion — but its concealment applies).
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
}
