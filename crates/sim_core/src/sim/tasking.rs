//! Belief-driven sensor tasking: where should each sensor look next?
//! Spec: `docs/DESIGN.md` §10.3. Gates: V57.
//!
//! This is the piece that finally makes the POMDP layer part of the simulation rather
//! than a display. Until now `pomdp.rs` was computed for an overlay and no sim code read
//! it; here each side keeps a belief over where the enemy might be, and points its
//! steerable sensors to learn the most.
//!
//! # The objective
//!
//! For a candidate facing, the observation is binary per cell: either a sensor detects
//! something at cell `c` (probability `b(c)·p(c)`), collapsing the belief to a point, or
//! it sees nothing and the belief becomes `b'(c) ∝ b(c)(1 − p(c))`. So
//!
//! ```text
//! E[H after] = (1 − Σ b(c)p(c)) · H(b')
//! gain       = H(b) − E[H after]
//! ```
//!
//! and the sensor takes the facing with the greatest gain. That is the real
//! information-gain control, not a proxy for it - and it is why a sensor prefers to sweep
//! *plausible* ground over ground it has already cleared, without being told to.
//!
//! # Why it is affordable
//!
//! The expensive part of a detection rate is the line-of-sight walk, and **LOS does not
//! depend on where a sensor is facing** - only the field-of-regard gate does. So the
//! per-cell rate is computed once per sensor, ignoring facing, and cached against the
//! pose it was built for; evaluating twelve candidate facings is then twelve cheap arc
//! masks over that raster. Without this the layer would cost a full viewshed per facing
//! per epoch and be unusable.
//!
//! Everything here is deterministic and draws no randomness.

use super::{SensorState, Side, Sim};
use crate::pomdp::SpatialBelief;
use crate::sensing::{self, p_detect_tick, SensorType};
use crate::terrain::TerrainGrid;
use glam::Vec2;
use ndarray::{Array2, Zip};

/// Candidate facings a steerable sensor chooses between, evenly spaced over the circle.
/// Twelve is a 30° step: fine enough to matter, coarse enough to stay cheap.
const CANDIDATE_FACINGS: usize = 12;

/// How much of each cell's belief mass leaks to its neighbours per epoch, modelling an
/// unobserved enemy that may have moved.
const DIFFUSION_PER_EPOCH: f32 = 0.12;

/// Altitude band a **carried** sensor's cached coverage is keyed on, metres.
///
/// See [`Sim::cache_pose`]: a drone's height above ground changes every tick as the ground
/// under it changes, so an exact key would never hit. 25 m is well inside the scale over
/// which a slant-range detection rate varies for an airborne observer.
const CARRIED_HEIGHT_STEP_M: f32 = 25.0;

/// One sensor's facing-independent coverage, and the pose it was computed for.
struct Coverage {
    pos: Vec2,
    height_m: f32,
    /// Detection rate per coarse cell, with the field-of-regard gate *not* applied.
    rate: Array2<f32>,
}

/// A side's belief about the other side, plus the cached coverage its sensors provide.
pub(super) struct Tasking {
    /// Coarse grid edge length, in cells.
    cells: usize,
    /// What Blue believes about Red, and vice versa.
    blue: SpatialBelief,
    red: SpatialBelief,
    /// Indexed by sensor. `None` until first built, or after the sensor moves.
    coverage: Vec<Option<Coverage>>,
}

impl Tasking {
    /// A fresh, maximally-uncertain belief for both sides.
    /// The coarse grid's edge length. Shared with the movement planner (§10.5), which
    /// plans at the same resolution for the same reason: a decision taken every ten seconds
    /// does not need ten-metre cells.
    pub(super) fn cells(&self) -> usize {
        self.cells
    }

    pub(super) fn new(cells: usize) -> Self {
        Self {
            cells,
            blue: SpatialBelief::uniform(cells, cells),
            red: SpatialBelief::uniform(cells, cells),
            coverage: Vec::new(),
        }
    }

    /// What `side` believes about its enemy.
    pub(super) fn belief(&self, side: Side) -> &SpatialBelief {
        match side {
            Side::Blue => &self.blue,
            Side::Red => &self.red,
        }
    }

    fn belief_mut(&mut self, side: Side) -> &mut SpatialBelief {
        match side {
            Side::Blue => &mut self.blue,
            Side::Red => &mut self.red,
        }
    }

    /// Forget everything: a fresh trial starts knowing nothing, and the cached coverage
    /// is indexed by sensor position in a list about to be rebuilt.
    pub(super) fn reset(&mut self) {
        self.blue = SpatialBelief::uniform(self.cells, self.cells);
        self.red = SpatialBelief::uniform(self.cells, self.cells);
        self.coverage.clear();
    }
}

impl Sim {
    /// The world position of a coarse belief cell's centre.
    fn coarse_centre(terrain: &TerrainGrid, cells: usize, ix: usize, iy: usize) -> Vec2 {
        let extent_x = terrain.width() as f32 * terrain.transform().cell_size_m();
        let extent_y = terrain.height() as f32 * terrain.transform().cell_size_m();
        Vec2::new(
            (ix as f32 + 0.5) / cells as f32 * extent_x,
            (iy as f32 + 0.5) / cells as f32 * extent_y,
        )
    }

    /// The pose a sensor's coverage raster is cached against.
    ///
    /// **Emplaced sensors use their exact pose.** Anything else would be an approximation
    /// where none is needed - they do not move, so the cache hits every epoch after the
    /// first, and exactness keeps V57 pinned to the real geometry.
    ///
    /// **Carried sensors are quantised to the coarse belief grid.** A raster costs `cells²`
    /// line-of-sight walks, affordable precisely because an emplaced sensor pays it once. A
    /// drone moves every tick, so an exact key would rebuild in full every epoch and never
    /// hit - which is why carried sensors used to be excluded from belief altogether. But
    /// the raster *is* a coarse-grid object: every entry is already a rate at a coarse cell
    /// centre. Keying it on the coarse cell the sensor is standing in is therefore
    /// consistent with the resolution the whole layer runs at, not a fudge, and it makes
    /// the cost proportional to how far the drone has flown rather than to how long it has
    /// been airborne.
    ///
    /// Quantisation is exact integer arithmetic, so this stays deterministic: the same
    /// flight path produces the same rebuild schedule on every run.
    fn cache_pose(&self, s_idx: usize, cells: usize) -> (Vec2, f32) {
        let (pos, height, _) = self.sensor_view(s_idx);
        if self.sensors[s_idx].carrier.is_none() {
            return (pos, height);
        }
        let extent_x = self.terrain.width() as f32 * self.terrain.transform().cell_size_m();
        let extent_y = self.terrain.height() as f32 * self.terrain.transform().cell_size_m();
        let cell_of = |v: f32, extent: f32| -> usize {
            let frac = (v / extent.max(f32::EPSILON)).clamp(0.0, 1.0);
            ((frac * cells as f32) as usize).min(cells.saturating_sub(1))
        };
        let quantised = Self::coarse_centre(
            &self.terrain,
            cells,
            cell_of(pos.x, extent_x),
            cell_of(pos.y, extent_y),
        );
        let band = (height / CARRIED_HEIGHT_STEP_M).round() * CARRIED_HEIGHT_STEP_M;
        (quantised, band)
    }

    /// Update each side's belief, then point every steerable sensor where it will learn
    /// the most (`docs/DESIGN.md` §10.3). Runs at the decision epoch.
    ///
    /// Deterministic: no randomness is drawn, and sensors are visited in index order.
    pub(super) fn task_sensors(&mut self) {
        if !self.sensor_tasking {
            return;
        }
        let mut tasking = std::mem::take(&mut self.tasking);
        let cells = tasking.cells;
        tasking.coverage.resize_with(self.sensors.len(), || None);

        // Refresh any coverage raster whose sensor has moved (or was never built).
        for s_idx in 0..self.sensors.len() {
            if !self.sensor_active(s_idx) {
                continue;
            }
            let (pos, height) = self.cache_pose(s_idx, cells);
            let stale = tasking.coverage[s_idx]
                .as_ref()
                .is_none_or(|c| c.pos != pos || c.height_m != height);
            if stale {
                tasking.coverage[s_idx] = Some(Coverage {
                    pos,
                    height_m: height,
                    rate: self.coverage_raster(s_idx, pos, height, cells),
                });
            }
        }

        for side in [Side::Blue, Side::Red] {
            self.update_belief(&mut tasking, side);
            self.choose_facings(&mut tasking, side);
        }

        self.tasking = tasking;
    }

    /// Detection rate against a reference target at every coarse cell, **ignoring the
    /// field of regard** - that gate is what the facing decision applies afterwards.
    ///
    /// Parallel over cells, and deterministic for the same reason the viewshed is: each
    /// cell writes its own slot and the LOS scratch is thread-local.
    fn coverage_raster(&self, s_idx: usize, pos: Vec2, height: f32, cells: usize) -> Array2<f32> {
        let sensor = &self.sensors[s_idx];
        // A representative enemy: the most conspicuous type the other side is fielding,
        // so tasking reasons about a target it could actually find.
        let (signature, target_height) = self.reference_target(sensor.side, sensor);
        let mut raster = Array2::<f32>::zeros((cells, cells));
        let terrain = &self.terrain;
        Zip::indexed(&mut raster).par_for_each(|(iy, ix), v| {
            let cell = Self::coarse_centre(terrain, cells, ix, iy);
            // Facing is irrelevant here: an all-round window is passed so the arc gate
            // never fires, and `choose_facings` applies the real arc per candidate.
            *v = sensing::detection_rate_against(
                terrain,
                &all_round(&sensor.stats),
                pos,
                height,
                0.0,
                cell,
                target_height,
                signature,
                sensing::concealment_at(terrain, cell),
            );
        });
        raster
    }

    /// Signature and height of the enemy this sensor should reason about: the most
    /// conspicuous live enemy unit, or a plain default when the side is empty.
    fn reference_target(&self, own_side: Side, sensor: &SensorState) -> (f32, f32) {
        self.units
            .iter()
            .filter(|u| u.side != own_side && u.alive())
            .map(|u| {
                (
                    u.stats.signature_in(sensor.stats.modality),
                    u.stats.height_m,
                )
            })
            .fold(
                (0.0f32, 2.0f32),
                |best, cur| {
                    if cur.0 > best.0 {
                        cur
                    } else {
                        best
                    }
                },
            )
    }

    /// Fold this epoch's *negative* information into a side's belief, then diffuse it.
    ///
    /// Only sensors that saw nothing contribute: a cell one of them covers well is a cell
    /// the enemy probably is not in, so belief drains out of covered ground and pools in
    /// dead ground and jammed areas.
    fn update_belief(&self, tasking: &mut Tasking, side: Side) {
        let cells = tasking.cells;
        let mut likelihood = Array2::<f32>::ones((cells, cells));
        let mut any = false;
        for s_idx in 0..self.sensors.len() {
            let sensor = &self.sensors[s_idx];
            if sensor.side != side || !self.sensor_active(s_idx) {
                continue;
            }
            let Some(cov) = tasking.coverage[s_idx].as_ref() else {
                continue;
            };
            any = true;
            let facing = sensor.facing_deg;
            let width = sensor.stats.for_width_deg;
            let pos = cov.pos;
            let terrain = &self.terrain;
            Zip::indexed(&mut likelihood)
                .and(&cov.rate)
                .for_each(|(iy, ix), l, &rate| {
                    let cell = Self::coarse_centre(terrain, cells, ix, iy);
                    let lambda = rate * arc_gate(pos, cell, facing, width);
                    *l *= 1.0 - p_detect_tick(lambda, self.epoch_s);
                });
        }
        if !any {
            return;
        }
        let belief = tasking.belief_mut(side);
        belief.update(&likelihood);
        // An unobserved enemy may have moved; diffusion is what stops the belief becoming
        // falsely confident about ground nobody has looked at for a while.
        belief.predict(DIFFUSION_PER_EPOCH);
    }

    /// Point every steerable sensor at the facing with the greatest expected information
    /// gain. A sensor with no field of regard sees all round, so there is nothing to
    /// choose.
    fn choose_facings(&mut self, tasking: &mut Tasking, side: Side) {
        let cells = tasking.cells;
        for s_idx in 0..self.sensors.len() {
            let sensor = &self.sensors[s_idx];
            if sensor.side != side || !self.sensor_active(s_idx) {
                continue;
            }
            // Carried sensors face where their airframe is pointing; steering them here
            // would be overwritten by `sync_carried_sensors` next tick anyway.
            if sensor.carrier.is_some() {
                continue;
            }
            let Some(width) = sensor.stats.for_width_deg else {
                continue; // all-round: no decision to make
            };
            let Some(cov) = tasking.coverage[s_idx].as_ref() else {
                continue;
            };

            let belief = tasking.belief(side).belief();
            let prior_entropy = tasking.belief(side).entropy();
            let mut best = (sensor.facing_deg, f32::NEG_INFINITY);
            for k in 0..CANDIDATE_FACINGS {
                #[allow(clippy::cast_precision_loss)]
                let facing = k as f32 * 360.0 / CANDIDATE_FACINGS as f32;
                let gain = self.expected_gain(
                    cells,
                    belief,
                    &cov.rate,
                    cov.pos,
                    facing,
                    Some(width),
                    prior_entropy,
                );
                // `>` keeps the first of equal maxima, so ties break on the lower facing.
                if gain > best.1 {
                    best = (facing, gain);
                }
            }
            self.sensors[s_idx].facing_deg = best.0;
        }
    }

    /// Expected reduction in belief entropy from looking along `facing` for one epoch.
    ///
    /// See the module header for the derivation. `O(cells²)` arithmetic - no LOS, because
    /// `rate` already carries it.
    #[allow(clippy::too_many_arguments)]
    fn expected_gain(
        &self,
        cells: usize,
        belief: &Array2<f32>,
        rate: &Array2<f32>,
        sensor_pos: Vec2,
        facing: f32,
        width: Option<f32>,
        prior_entropy: f32,
    ) -> f32 {
        let mut p_detect_any = 0.0f32; // Σ b(c)·p(c)
        let mut miss_mass = 0.0f32; // Σ b(c)(1 − p(c)), the normaliser of b'
        let mut miss_entropy_terms = Vec::with_capacity(cells * cells);
        for ((iy, ix), &b) in belief.indexed_iter() {
            if b <= 0.0 {
                continue;
            }
            let cell = Self::coarse_centre(&self.terrain, cells, ix, iy);
            let lambda = rate[[iy, ix]] * arc_gate(sensor_pos, cell, facing, width);
            let p = p_detect_tick(lambda, self.epoch_s);
            p_detect_any += b * p;
            let m = b * (1.0 - p);
            miss_mass += m;
            miss_entropy_terms.push(m);
        }
        if miss_mass <= 0.0 {
            return prior_entropy; // certain detection: all uncertainty resolved
        }
        // H(b') for the renormalised no-detection posterior.
        let posterior_entropy: f32 = miss_entropy_terms
            .iter()
            .filter(|&&m| m > 0.0)
            .map(|&m| {
                let q = m / miss_mass;
                -q * q.ln()
            })
            .sum();
        // A detection collapses the belief to a point, so contributes zero entropy.
        prior_entropy - (1.0 - p_detect_any) * posterior_entropy
    }

    /// A side's current belief about the other, for the app's overlay and for gates.
    #[must_use]
    pub fn belief_of(&self, side: Side) -> &SpatialBelief {
        self.tasking.belief(side)
    }
}

/// 1 when `cell` lies inside the sensor's arc, 0 outside. An all-round sensor is always 1.
fn arc_gate(sensor_pos: Vec2, cell: Vec2, facing_deg: f32, width_deg: Option<f32>) -> f32 {
    let Some(width) = width_deg else {
        return 1.0;
    };
    let to = cell - sensor_pos;
    if to.length_squared() < 1e-6 {
        return 1.0;
    }
    let bearing = to.y.atan2(to.x).to_degrees();
    let mut off = (bearing - facing_deg) % 360.0;
    if off > 180.0 {
        off -= 360.0;
    } else if off < -180.0 {
        off += 360.0;
    }
    f32::from(off.abs() <= width * 0.5)
}

/// The same sensor with its field of regard removed, so one coverage raster can be built
/// and re-gated per candidate facing.
fn all_round(s: &SensorType) -> SensorType {
    SensorType {
        for_width_deg: None,
        ..s.clone()
    }
}

impl Default for Tasking {
    fn default() -> Self {
        Self::new(1)
    }
}
