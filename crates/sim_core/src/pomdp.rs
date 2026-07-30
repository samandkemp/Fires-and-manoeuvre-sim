//! Partial observability: belief-state estimation over enemy position (`docs/DESIGN.md`
//! §8). With EW degrading detection, an observer never knows the truth — it maintains a
//! probability distribution and updates it by Bayes' rule. Validated on the canonical
//! Tiger problem (V41) and on spatial "negative information" (V42–V43).
//!
//! The belief is an *inference layer* over the sim, not sim state — so it lives here as a
//! standalone tool the app/experiments drive from detection events and their absence.

use crate::ew::{jamming_factor, Jammer};
use crate::sensing::{detection_rate, p_detect_tick, SensorType, UnitType};
use crate::terrain::TerrainGrid;
use glam::Vec2;
use ndarray::Array2;

/// Discrete Bayes update: posterior ∝ prior · likelihood, renormalised. If the evidence
/// is impossible under the prior (all-zero product) the prior is returned unchanged.
///
/// # Panics
/// If `prior` and `likelihood` differ in length.
#[must_use]
pub fn bayes_update(prior: &[f32], likelihood: &[f32]) -> Vec<f32> {
    assert_eq!(
        prior.len(),
        likelihood.len(),
        "prior and likelihood must match"
    );
    let mut post: Vec<f32> = prior.iter().zip(likelihood).map(|(p, l)| p * l).collect();
    let sum: f32 = post.iter().sum();
    if sum > 0.0 {
        for p in &mut post {
            *p /= sum;
        }
        post
    } else {
        prior.to_vec()
    }
}

/// A belief distribution over enemy position, one probability per terrain cell.
#[derive(Clone, Debug)]
pub struct SpatialBelief {
    belief: Array2<f32>,
}

impl SpatialBelief {
    /// A maximally-uncertain belief: uniform over all `width × height` cells.
    #[must_use]
    pub fn uniform(width: usize, height: usize) -> Self {
        let p = 1.0 / (width * height) as f32;
        Self {
            belief: Array2::from_elem((height, width), p),
        }
    }

    /// The belief raster (rows = northing, sums to 1).
    #[must_use]
    pub fn belief(&self) -> &Array2<f32> {
        &self.belief
    }

    /// Bayes-update the belief by a per-cell observation likelihood, then renormalise.
    ///
    /// # Panics
    /// If `likelihood`'s shape differs from the belief's.
    pub fn update(&mut self, likelihood: &Array2<f32>) {
        assert_eq!(
            likelihood.dim(),
            self.belief.dim(),
            "likelihood shape must match"
        );
        self.belief *= likelihood;
        let sum = self.belief.sum();
        if sum > 0.0 {
            self.belief /= sum;
        }
    }

    /// Motion model: diffuse `rate ∈ [0,1]` of each cell's mass into its 4-neighbours,
    /// modelling that an unobserved target may have moved. Raises entropy.
    pub fn predict(&mut self, rate: f32) {
        let (h, w) = self.belief.dim();
        let mut next = Array2::zeros((h, w));
        for iy in 0..h {
            for ix in 0..w {
                let mut acc = 0.0f32;
                let mut count = 0.0f32;
                for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                    let nx = ix as isize + dx;
                    let ny = iy as isize + dy;
                    if nx >= 0 && ny >= 0 && nx < w as isize && ny < h as isize {
                        acc += self.belief[[ny as usize, nx as usize]];
                        count += 1.0;
                    }
                }
                let neighbour_avg = if count > 0.0 {
                    acc / count
                } else {
                    self.belief[[iy, ix]]
                };
                next[[iy, ix]] = (1.0 - rate) * self.belief[[iy, ix]] + rate * neighbour_avg;
            }
        }
        let sum = next.sum();
        if sum > 0.0 {
            next /= sum;
        }
        self.belief = next;
    }

    /// The most probable cell (`ix`, `iy`).
    #[must_use]
    pub fn most_likely_cell(&self) -> (usize, usize) {
        let mut best = (0, 0);
        let mut best_p = f32::NEG_INFINITY;
        for ((iy, ix), &p) in self.belief.indexed_iter() {
            if p > best_p {
                best_p = p;
                best = (ix, iy);
            }
        }
        best
    }

    /// Shannon entropy (nats) — total uncertainty; 0 when the belief is a point mass.
    #[must_use]
    pub fn entropy(&self) -> f32 {
        -self
            .belief
            .iter()
            .filter(|&&p| p > 0.0)
            .map(|&p| p * p.ln())
            .sum::<f32>()
    }

    /// Total belief mass over the cells satisfying `pred(ix, iy)`.
    pub fn mass_where(&self, pred: impl Fn(usize, usize) -> bool) -> f32 {
        self.belief
            .indexed_iter()
            .filter(|((iy, ix), _)| pred(*ix, *iy))
            .map(|(_, &p)| p)
            .sum()
    }
}

/// The negative-information observation likelihood `P(no detection this tick | enemy of
/// `target` type at each cell)` for one sensor, including EW: cells the sensor covers
/// well get low likelihood (the enemy would have been seen), dead ground / jammed cells
/// get ~1. Multiplying a uniform prior by this is "where an *undetected* enemy could be".
#[must_use]
pub fn no_detection_likelihood(
    terrain: &TerrainGrid,
    sensor: &SensorType,
    sensor_pos: Vec2,
    facing_deg: f32,
    target: &UnitType,
    jammers: &[Jammer],
    dt_s: f32,
) -> Array2<f32> {
    let (h, w) = (terrain.height(), terrain.width());
    Array2::from_shape_fn((h, w), |(iy, ix)| {
        let cell = terrain.transform().cell_center(ix, iy);
        let lambda = detection_rate(terrain, sensor, sensor_pos, facing_deg, target, cell)
            * jamming_factor(cell, jammers);
        1.0 - p_detect_tick(lambda, dt_s)
    })
}
