//! Terrain: the elevation raster, terrain-type layer, and the derived cover /
//! concealment / mobility layers, plus the world↔grid transform. See `docs/DESIGN.md`
//! §1. Line of sight (which reads this) arrives in plan step 1.3.

use crate::SimRng;
use glam::Vec2;
use ndarray::Array2;
use rand::{Rng, SeedableRng};

/// How a cell is classified. Drives cover, concealment, mobility, and how the cell
/// blocks or attenuates line of sight (`docs/DESIGN.md` §1.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum TerrainType {
    /// Bare, trafficable ground: no blocking feature, no concealment.
    #[default]
    Open,
    /// Woodland: a canopy that *attenuates* sightlines (soft) and conceals.
    Trees,
    /// Built-up: buildings that *hard-block* sightlines and give strong cover.
    Urban,
}

/// The tweakable dials for one terrain type. Abstract placeholders, loaded from
/// `scenarios/terrain_types.toml` — the models are the product, these are the knobs.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct TerrainParams {
    /// Canopy / building height above ground, metres — the blocking surface is `z + f`.
    pub feature_height_m: f32,
    /// κ: sight attenuation per metre of canopy; transmittance `τ = exp(−κ·L)`.
    pub extinction_per_m: f32,
    /// Protection against fires, `[0, 1]`.
    pub cover: f32,
    /// Reduction in detectability at the cell, `[0, 1]`.
    pub concealment: f32,
    /// Movement-cost multiplier, `≥ 1` (`f32::INFINITY` = impassable).
    pub mobility_cost: f32,
}

/// The dials for every terrain type, keyed by name in `terrain_types.toml`.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct TerrainParamsTable {
    /// Dials for [`TerrainType::Open`].
    pub open: TerrainParams,
    /// Dials for [`TerrainType::Trees`].
    pub trees: TerrainParams,
    /// Dials for [`TerrainType::Urban`].
    pub urban: TerrainParams,
}

impl TerrainParamsTable {
    /// The dials for a given terrain type.
    #[must_use]
    pub fn get(&self, t: TerrainType) -> TerrainParams {
        match t {
            TerrainType::Open => self.open,
            TerrainType::Trees => self.trees,
            TerrainType::Urban => self.urban,
        }
    }
}

/// The single place world↔cell conversion happens (`docs/DESIGN.md` §1.1).
///
/// World frame: metres, X east, Y north; grid origin at the south-west corner; `ix`
/// increases east, `iy` increases north. Values are registered at cell centres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridTransform {
    cell_size_m: f32,
    width: usize,
    height: usize,
}

impl GridTransform {
    /// A transform for a `width × height` grid of square `cell_size_m` cells.
    #[must_use]
    pub fn new(cell_size_m: f32, width: usize, height: usize) -> Self {
        Self {
            cell_size_m,
            width,
            height,
        }
    }

    /// Cells east (the `ix` extent).
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Cells north (the `iy` extent).
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Cell size, metres.
    #[must_use]
    pub fn cell_size_m(&self) -> f32 {
        self.cell_size_m
    }

    /// World position of the centre of cell `(ix, iy)`.
    #[must_use]
    pub fn cell_center(&self, ix: usize, iy: usize) -> Vec2 {
        Vec2::new(
            (ix as f32 + 0.5) * self.cell_size_m,
            (iy as f32 + 0.5) * self.cell_size_m,
        )
    }

    /// The integer cell containing `world`, or `None` if it lies outside the grid.
    #[must_use]
    pub fn world_to_cell(&self, world: Vec2) -> Option<(usize, usize)> {
        if world.x < 0.0 || world.y < 0.0 {
            return None;
        }
        let ix = (world.x / self.cell_size_m).floor() as usize;
        let iy = (world.y / self.cell_size_m).floor() as usize;
        if ix < self.width && iy < self.height {
            Some((ix, iy))
        } else {
            None
        }
    }

    /// Fractional cell coordinates: the value whose integer part indexes the cell
    /// centre at-or-below `world`, used for bilinear interpolation. `centre(ix,iy)`
    /// maps to exactly `(ix, iy)`.
    #[must_use]
    pub fn world_to_frac(&self, world: Vec2) -> (f32, f32) {
        (
            world.x / self.cell_size_m - 0.5,
            world.y / self.cell_size_m - 0.5,
        )
    }
}

/// The terrain: elevation, type, and the derived layers, all sharing one grid.
///
/// Built once (shape invariants are enforced at construction); read everywhere.
#[derive(Clone, Debug)]
pub struct TerrainGrid {
    transform: GridTransform,
    elevation_m: Array2<f32>,
    terrain_type: Array2<TerrainType>,
    feature_height_m: Array2<f32>,
    extinction_per_m: Array2<f32>,
    cover: Array2<f32>,
    concealment: Array2<f32>,
    mobility_cost: Array2<f32>,
}

impl TerrainGrid {
    /// Build a grid from an elevation and a terrain-type layer, precomputing the derived
    /// layers from `params`.
    ///
    /// # Panics
    /// If the two layers have different shapes — that is a construction bug, not user
    /// input (sources and the loader always produce matching shapes).
    #[must_use]
    pub fn from_layers(
        cell_size_m: f32,
        elevation_m: Array2<f32>,
        terrain_type: Array2<TerrainType>,
        params: &TerrainParamsTable,
    ) -> Self {
        assert_eq!(
            elevation_m.dim(),
            terrain_type.dim(),
            "elevation and terrain-type layers must have the same shape"
        );
        let (h, w) = elevation_m.dim();
        let transform = GridTransform::new(cell_size_m, w, h);

        // `from_shape_fn` fills each cell from a closure — idiomatic ndarray, and it
        // keeps the derived layers provably in lock-step with the type layer.
        let feature_height_m = Array2::from_shape_fn((h, w), |(iy, ix)| {
            params.get(terrain_type[[iy, ix]]).feature_height_m
        });
        let extinction_per_m = Array2::from_shape_fn((h, w), |(iy, ix)| {
            params.get(terrain_type[[iy, ix]]).extinction_per_m
        });
        let cover =
            Array2::from_shape_fn((h, w), |(iy, ix)| params.get(terrain_type[[iy, ix]]).cover);
        let concealment = Array2::from_shape_fn((h, w), |(iy, ix)| {
            params.get(terrain_type[[iy, ix]]).concealment
        });
        let mobility_cost = Array2::from_shape_fn((h, w), |(iy, ix)| {
            params.get(terrain_type[[iy, ix]]).mobility_cost
        });

        Self {
            transform,
            elevation_m,
            terrain_type,
            feature_height_m,
            extinction_per_m,
            cover,
            concealment,
            mobility_cost,
        }
    }

    /// The world↔cell transform.
    #[must_use]
    pub fn transform(&self) -> &GridTransform {
        &self.transform
    }

    /// Cells east.
    #[must_use]
    pub fn width(&self) -> usize {
        self.transform.width
    }

    /// Cells north.
    #[must_use]
    pub fn height(&self) -> usize {
        self.transform.height
    }

    /// Bare-earth elevation `z` at each cell centre.
    #[must_use]
    pub fn elevation(&self) -> &Array2<f32> {
        &self.elevation_m
    }

    /// Terrain type of each cell.
    #[must_use]
    pub fn terrain_type(&self) -> &Array2<TerrainType> {
        &self.terrain_type
    }

    /// Feature height `f` (canopy / building above ground) at each cell.
    #[must_use]
    pub fn feature_height(&self) -> &Array2<f32> {
        &self.feature_height_m
    }

    /// Canopy extinction κ (per metre) at each cell; 0 where nothing attenuates.
    #[must_use]
    pub fn extinction(&self) -> &Array2<f32> {
        &self.extinction_per_m
    }

    /// Cover layer, `[0, 1]`.
    #[must_use]
    pub fn cover(&self) -> &Array2<f32> {
        &self.cover
    }

    /// Concealment layer, `[0, 1]`.
    #[must_use]
    pub fn concealment(&self) -> &Array2<f32> {
        &self.concealment
    }

    /// Mobility-cost layer, `≥ 1`.
    #[must_use]
    pub fn mobility_cost(&self) -> &Array2<f32> {
        &self.mobility_cost
    }

    /// Terrain type of the cell containing `world`, or [`TerrainType::Open`] if outside.
    #[must_use]
    pub fn terrain_at(&self, world: Vec2) -> TerrainType {
        match self.transform.world_to_cell(world) {
            Some((ix, iy)) => self.terrain_type[[iy, ix]],
            None => TerrainType::Open,
        }
    }

    /// Movement cost along one grid edge, from cell `from` to 8-neighbour cell `to`.
    ///
    /// Cost = horizontal distance × the mean terrain mobility multiplier of the two
    /// cells × a slope factor that penalises uphill grades harder than downhill —
    /// Phase 5's DP paths over cell *edges* so slope direction matters.
    /// `INFINITY` where either cell is impassable.
    ///
    /// The slope-penalty constants are placeholder dials; they move into the movement
    /// TOML when Phase 5 formalises the movement model.
    ///
    /// # Panics
    /// If the cells are not distinct 8-neighbours — callers iterate neighbourhoods, so
    /// a non-adjacent pair is a bug, not data.
    #[must_use]
    pub fn move_cost(&self, from: (usize, usize), to: (usize, usize)) -> f32 {
        let dx = from.0.abs_diff(to.0);
        let dy = from.1.abs_diff(to.1);
        assert!(
            dx <= 1 && dy <= 1 && dx + dy > 0,
            "move_cost is defined on 8-neighbour edges"
        );

        // Placeholder dials (→ movement TOML in Phase 5).
        const UPHILL_PENALTY: f32 = 4.0;
        const DOWNHILL_PENALTY: f32 = 1.5;

        let m_from = self.mobility_cost[[from.1, from.0]];
        let m_to = self.mobility_cost[[to.1, to.0]];
        if !m_from.is_finite() || !m_to.is_finite() {
            return f32::INFINITY;
        }

        let dist = self.transform.cell_size_m
            * if dx + dy == 2 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
        let dz = self.elevation_m[[to.1, to.0]] - self.elevation_m[[from.1, from.0]];
        let grade = dz / dist; // rise over run; positive = uphill
        let slope_factor =
            1.0 + UPHILL_PENALTY * grade.max(0.0) + DOWNHILL_PENALTY * (-grade).max(0.0);

        dist * 0.5 * (m_from + m_to) * slope_factor
    }

    /// Bilinearly-interpolated bare-earth elevation `z` at an arbitrary world position.
    ///
    /// Exact for affine surfaces (validation V2). Positions outside the cell-centre hull
    /// clamp to the edge value (flat extrapolation) rather than extrapolating a slope.
    #[must_use]
    pub fn sample_elevation(&self, world: Vec2) -> f32 {
        let (fx, fy) = self.transform.world_to_frac(world);
        // Clamp into the cell-centre grid *before* splitting into cell + fraction, so an
        // out-of-hull position resolves to a genuine edge value with fraction 0.
        let fx = fx.clamp(0.0, self.width() as f32 - 1.0);
        let fy = fy.clamp(0.0, self.height() as f32 - 1.0);

        let ix0 = fx.floor() as usize;
        let iy0 = fy.floor() as usize;
        let ix1 = (ix0 + 1).min(self.width() - 1);
        let iy1 = (iy0 + 1).min(self.height() - 1);
        let tx = fx - ix0 as f32;
        let ty = fy - iy0 as f32;

        let e = &self.elevation_m;
        let top = lerp(e[[iy0, ix0]], e[[iy0, ix1]], tx);
        let bot = lerp(e[[iy1, ix0]], e[[iy1, ix1]], tx);
        lerp(top, bot, ty)
    }
}

/// Linear interpolation `a + (b − a)·t`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A recipe for generating a [`TerrainGrid`]'s elevation. Externally tagged in TOML
/// (`[terrain.source.hills]`), so new sources are additive (`docs/DESIGN.md` §1.3).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainSource {
    /// Constant elevation, all [`TerrainType::Open`] — the simplest fixture.
    Flat {
        /// The elevation, metres.
        elevation_m: f32,
    },
    /// Seeded rolling relief: a sum of `count` Gaussian hills placed by the RNG, with
    /// woodland painted where a second seeded field exceeds its quantile and a few
    /// rectangular urban blocks.
    Hills {
        /// Number of hills.
        count: u32,
        /// Peak height of the tallest hill, metres (each is a random fraction of this).
        max_height_m: f32,
        /// Characteristic hill radius (σ), metres (each is randomised around this).
        base_radius_m: f32,
        /// Fraction of the map painted `Trees` (by quantile of a woods field), `[0, 1)`.
        #[serde(default = "default_woods_fraction")]
        woods_fraction: f32,
        /// Number of rectangular `Urban` blocks scattered on the map.
        #[serde(default = "default_urban_blocks")]
        urban_blocks: u32,
    },
}

fn default_woods_fraction() -> f32 {
    0.25
}

fn default_urban_blocks() -> u32 {
    3
}

impl TerrainSource {
    /// Build a grid of `width × height` cells at `cell_size_m`, seeded by `seed`.
    ///
    /// Deterministic: identical `(source, dimensions, seed)` → bit-identical elevation.
    #[must_use]
    pub fn build(
        &self,
        cell_size_m: f32,
        width: usize,
        height: usize,
        seed: u64,
        params: &TerrainParamsTable,
    ) -> TerrainGrid {
        match *self {
            TerrainSource::Flat { elevation_m } => TerrainGrid::from_layers(
                cell_size_m,
                Array2::from_elem((height, width), elevation_m),
                Array2::from_elem((height, width), TerrainType::Open),
                params,
            ),
            TerrainSource::Hills {
                count,
                max_height_m,
                base_radius_m,
                woods_fraction,
                urban_blocks,
            } => {
                let transform = GridTransform::new(cell_size_m, width, height);
                // One RNG stream, consumed in a fixed order (relief → woods → urban),
                // keeps the whole terrain a single deterministic function of the seed.
                let mut rng = SimRng::seed_from_u64(seed);

                let hills = place_hills(count, max_height_m, base_radius_m, &transform, &mut rng);
                let elevation_m = Array2::from_shape_fn((height, width), |(iy, ix)| {
                    let p = transform.cell_center(ix, iy);
                    hills.iter().map(|hill| hill.elevation_at(p)).sum()
                });

                // Woods: an independent hill-sum field, thresholded at the quantile that
                // paints the requested fraction of cells.
                let woods_hills =
                    place_hills(count * 3, 1.0, base_radius_m * 0.5, &transform, &mut rng);
                let woods_field = Array2::from_shape_fn((height, width), |(iy, ix)| {
                    let p = transform.cell_center(ix, iy);
                    woods_hills
                        .iter()
                        .map(|hill| hill.elevation_at(p))
                        .sum::<f32>()
                });
                let mut terrain_type = Array2::from_elem((height, width), TerrainType::Open);
                let frac = woods_fraction.clamp(0.0, 0.95);
                if frac > 0.0 {
                    let mut sorted: Vec<f32> = woods_field.iter().copied().collect();
                    sorted.sort_by(f32::total_cmp);
                    let idx = ((1.0 - frac) * (sorted.len() - 1) as f32).round() as usize;
                    let threshold = sorted[idx];
                    for (t, &v) in terrain_type.iter_mut().zip(woods_field.iter()) {
                        if v > threshold {
                            *t = TerrainType::Trees;
                        }
                    }
                }

                // Urban: rectangular blocks (~200–500 m across), overriding woods.
                let extent_x = width as f32 * cell_size_m;
                let extent_y = height as f32 * cell_size_m;
                for _ in 0..urban_blocks {
                    let cx = rng.random_range(0.0..extent_x);
                    let cy = rng.random_range(0.0..extent_y);
                    let half_x = rng.random_range(100.0..250.0f32);
                    let half_y = rng.random_range(100.0..250.0f32);
                    for iy in 0..height {
                        for ix in 0..width {
                            let p = transform.cell_center(ix, iy);
                            if (p.x - cx).abs() < half_x && (p.y - cy).abs() < half_y {
                                terrain_type[[iy, ix]] = TerrainType::Urban;
                            }
                        }
                    }
                }

                TerrainGrid::from_layers(cell_size_m, elevation_m, terrain_type, params)
            }
        }
    }
}

/// One Gaussian hill: `height · exp(−d² / 2σ²)`.
struct Hill {
    center: Vec2,
    height: f32,
    sigma: f32,
}

impl Hill {
    fn elevation_at(&self, p: Vec2) -> f32 {
        let d2 = (p - self.center).length_squared();
        self.height * (-d2 / (2.0 * self.sigma * self.sigma)).exp()
    }
}

/// Place `count` hills by drawing from `rng` in a fixed order, so the result is
/// reproducible for a given RNG state.
fn place_hills(
    count: u32,
    max_height_m: f32,
    base_radius_m: f32,
    transform: &GridTransform,
    rng: &mut SimRng,
) -> Vec<Hill> {
    let extent_x = transform.width() as f32 * transform.cell_size_m();
    let extent_y = transform.height() as f32 * transform.cell_size_m();
    (0..count)
        .map(|_| Hill {
            center: Vec2::new(
                rng.random_range(0.0..extent_x),
                rng.random_range(0.0..extent_y),
            ),
            height: rng.random_range(0.3..1.0) * max_height_m,
            sigma: rng.random_range(0.5..1.5) * base_radius_m,
        })
        .collect()
}
