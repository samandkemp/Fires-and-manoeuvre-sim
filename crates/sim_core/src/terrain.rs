//! The elevation raster, terrain types, the derived cover/concealment/mobility layers,
//! and the world↔grid transform. Spec: `docs/DESIGN.md` §1.

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
#[serde(deny_unknown_fields)]
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
    /// A composable recipe: a base surface plus ordered feature layers, which is how a
    /// map is *described* rather than picked from a menu (`docs/DESIGN.md` §1.3).
    Layers(TerrainRecipe),
    /// A named recipe — `{ preset = "mountain_pass" }` — expanded via
    /// [`TerrainPreset::recipe`].
    Preset(TerrainPreset),
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
                let elevation_m = hill_sum_field(&hills, &transform, width, height);

                // Woods: an independent hill-sum field, thresholded at the quantile that
                // paints the requested fraction of cells.
                let woods_hills =
                    place_hills(count * 3, 1.0, base_radius_m * 0.5, &transform, &mut rng);
                let woods_field = hill_sum_field(&woods_hills, &transform, width, height);
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
            // `Flat` and `Hills` above are kept as their own arms rather than folded into
            // recipes: they draw from the RNG in an order that scenarios and gates already
            // depend on, and re-expressing them would silently change every seeded map.
            TerrainSource::Layers(ref recipe) => {
                recipe.build(cell_size_m, width, height, seed, params)
            }
            TerrainSource::Preset(preset) => {
                preset
                    .recipe()
                    .build(cell_size_m, width, height, seed, params)
            }
        }
    }
}

/// A composable terrain recipe: a base surface, then ordered feature layers
/// (`docs/DESIGN.md` §1.3).
///
/// This is what lets a map be *described* — "rolling hills, a ridge through the middle,
/// light urban" — rather than picked from a fixed menu. Layers are applied **in the order
/// written**, each drawing from the one seeded RNG, so the listed order is part of the
/// determinism contract: the same recipe and seed always give the same map, and swapping
/// two layers is a different map (urban over woodland leaves urban).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainRecipe {
    /// The starting surface.
    pub base: BaseRelief,
    /// Features painted onto it, in order.
    #[serde(default)]
    pub apply: Vec<TerrainLayer>,
}

/// The surface a recipe starts from.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseRelief {
    /// Dead flat at a constant elevation.
    Flat {
        /// The elevation, metres.
        elevation_m: f32,
    },
    /// A sum of seeded Gaussian hills — the rolling-relief base.
    Hills {
        /// Number of hills.
        count: u32,
        /// Peak height of the tallest, metres.
        max_height_m: f32,
        /// Characteristic hill radius (σ), metres.
        base_radius_m: f32,
    },
}

/// One feature painted onto the base.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainLayer {
    /// A linear ridge across the whole map — "a mountain running through the middle".
    ///
    /// A Gaussian cross-section about a line through the map centre, so the crest is a
    /// smooth barrier rather than a wall: elevation gains
    /// `crest_m · exp(−d² / 2·(width_m/2)²)` for perpendicular distance `d`.
    Ridge {
        /// Direction the ridge line runs, degrees (0° = east, CCW).
        bearing_deg: f32,
        /// Height added at the crest, metres.
        crest_m: f32,
        /// Full width of the ridge, metres (the Gaussian's 2σ).
        width_m: f32,
        /// Perpendicular offset of the ridge line from the map centre, metres.
        #[serde(default)]
        offset_m: f32,
    },
    /// Woodland painted where a seeded field exceeds the quantile giving `fraction`.
    Woodland {
        /// Fraction of the map to paint as `Trees`, `[0, 1)`.
        fraction: f32,
        /// Characteristic patch size, metres — small means many copses, large means
        /// a few big forests at the same total coverage.
        #[serde(default = "default_patch_scale")]
        patch_scale_m: f32,
    },
    /// Rectangular urban blocks, overriding whatever they land on.
    Urban {
        /// Number of blocks.
        blocks: u32,
        /// Smallest block edge, metres.
        #[serde(default = "default_block_min")]
        min_size_m: f32,
        /// Largest block edge, metres.
        #[serde(default = "default_block_max")]
        max_size_m: f32,
    },
}

/// Ceiling on hills in a generated field. Terrain generation is `O(cells x hills)`, and
/// the woodland count scales with map area, so without a cap a large map silently costs
/// seconds. See [`TerrainLayer::Woodland`].
const MAX_FIELD_HILLS: u32 = 256;

fn default_patch_scale() -> f32 {
    300.0
}

fn default_block_min() -> f32 {
    200.0
}

fn default_block_max() -> f32 {
    500.0
}

/// A named recipe, so the common maps can be asked for by name.
///
/// Each expands to a [`TerrainRecipe`] — sugar, not a separate mechanism, so a preset can
/// always be copied out and adjusted.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainPreset {
    /// A featureless plain: the fixture for isolating a model from terrain.
    FlatPlain,
    /// Gentle relief and scattered copses — the default battlegroup map.
    RollingHills,
    /// Rolling relief under heavy forest: concealment everywhere, few sightlines.
    WoodedHills,
    /// A few small built-up areas on open ground.
    LightUrban,
    /// A dense built-up belt: hard LOS blocking and short engagement ranges.
    DenseUrban,
    /// A high ridge across the middle of an otherwise rolling map, with a wooded valley
    /// either side — the map that makes defilade and masking the whole problem.
    MountainPass,
}

impl TerrainPreset {
    /// Expand to the recipe it stands for.
    #[must_use]
    pub fn recipe(self) -> TerrainRecipe {
        let rolling = BaseRelief::Hills {
            count: 24,
            max_height_m: 120.0,
            base_radius_m: 600.0,
        };
        match self {
            Self::FlatPlain => TerrainRecipe {
                base: BaseRelief::Flat { elevation_m: 0.0 },
                apply: Vec::new(),
            },
            Self::RollingHills => TerrainRecipe {
                base: rolling,
                apply: vec![TerrainLayer::Woodland {
                    fraction: 0.25,
                    patch_scale_m: 300.0,
                }],
            },
            Self::WoodedHills => TerrainRecipe {
                base: rolling,
                apply: vec![TerrainLayer::Woodland {
                    fraction: 0.6,
                    patch_scale_m: 500.0,
                }],
            },
            Self::LightUrban => TerrainRecipe {
                base: rolling,
                apply: vec![
                    TerrainLayer::Woodland {
                        fraction: 0.2,
                        patch_scale_m: 300.0,
                    },
                    TerrainLayer::Urban {
                        blocks: 4,
                        min_size_m: 200.0,
                        max_size_m: 400.0,
                    },
                ],
            },
            Self::DenseUrban => TerrainRecipe {
                base: BaseRelief::Hills {
                    count: 10,
                    max_height_m: 40.0,
                    base_radius_m: 700.0,
                },
                apply: vec![
                    TerrainLayer::Woodland {
                        fraction: 0.08,
                        patch_scale_m: 200.0,
                    },
                    TerrainLayer::Urban {
                        blocks: 22,
                        min_size_m: 250.0,
                        max_size_m: 700.0,
                    },
                ],
            },
            Self::MountainPass => TerrainRecipe {
                base: rolling,
                apply: vec![
                    TerrainLayer::Ridge {
                        bearing_deg: 20.0,
                        crest_m: 320.0,
                        width_m: 1400.0,
                        offset_m: 0.0,
                    },
                    TerrainLayer::Woodland {
                        fraction: 0.35,
                        patch_scale_m: 400.0,
                    },
                ],
            },
        }
    }
}

impl TerrainRecipe {
    /// Build the recipe: base surface, then each layer in the order written, all drawing
    /// from one seeded stream.
    fn build(
        &self,
        cell_size_m: f32,
        width: usize,
        height: usize,
        seed: u64,
        params: &TerrainParamsTable,
    ) -> TerrainGrid {
        let transform = GridTransform::new(cell_size_m, width, height);
        let mut rng = SimRng::seed_from_u64(seed);

        let mut elevation_m = match self.base {
            BaseRelief::Flat { elevation_m } => Array2::from_elem((height, width), elevation_m),
            BaseRelief::Hills {
                count,
                max_height_m,
                base_radius_m,
            } => {
                let hills = place_hills(count, max_height_m, base_radius_m, &transform, &mut rng);
                hill_sum_field(&hills, &transform, width, height)
            }
        };
        let mut terrain_type = Array2::from_elem((height, width), TerrainType::Open);

        for layer in &self.apply {
            layer.apply(
                &mut elevation_m,
                &mut terrain_type,
                &transform,
                &mut rng,
                width,
                height,
            );
        }
        TerrainGrid::from_layers(cell_size_m, elevation_m, terrain_type, params)
    }
}

impl TerrainLayer {
    fn apply(
        &self,
        elevation_m: &mut Array2<f32>,
        terrain_type: &mut Array2<TerrainType>,
        transform: &GridTransform,
        rng: &mut SimRng,
        width: usize,
        height: usize,
    ) {
        let extent_x = width as f32 * transform.cell_size_m();
        let extent_y = height as f32 * transform.cell_size_m();
        let centre = Vec2::new(extent_x * 0.5, extent_y * 0.5);

        match *self {
            Self::Ridge {
                bearing_deg,
                crest_m,
                width_m,
                offset_m,
            } => {
                // Perpendicular distance to a line through `centre + offset·n` running
                // along `bearing_deg`; `n` is the unit normal to that direction.
                let dir = Vec2::from_angle(bearing_deg.to_radians());
                let normal = Vec2::new(-dir.y, dir.x);
                let sigma = (width_m * 0.5).max(1.0);
                for iy in 0..height {
                    for ix in 0..width {
                        let p = transform.cell_center(ix, iy);
                        let d = (p - centre).dot(normal) - offset_m;
                        elevation_m[[iy, ix]] += crest_m * (-(d * d) / (2.0 * sigma * sigma)).exp();
                    }
                }
            }
            Self::Woodland {
                fraction,
                patch_scale_m,
            } => {
                // A second hill-sum field thresholded at the quantile that paints the
                // requested fraction — patch_scale sets how clumped the result is.
                let frac = fraction.clamp(0.0, 0.95);
                if frac <= 0.0 {
                    return;
                }
                // One field hill per patch-sized area, **capped**: the count scales with
                // map area, so a 10 km map at a 300 m patch scale would ask for 1111
                // hills and the O(cells x hills) evaluation becomes a billion operations.
                // Past a few hundred the extra hills only average each other out — the
                // coverage is set by the quantile below, not by the count.
                let wanted = ((extent_x * extent_y) / (patch_scale_m * patch_scale_m)).max(1.0);
                let count = (wanted as u32).min(MAX_FIELD_HILLS);
                let field_hills = place_hills(count, 1.0, patch_scale_m * 0.5, transform, rng);
                let field = hill_sum_field(&field_hills, transform, width, height);
                let mut sorted: Vec<f32> = field.iter().copied().collect();
                sorted.sort_by(f32::total_cmp);
                let idx = ((1.0 - frac) * (sorted.len() - 1) as f32).round() as usize;
                let threshold = sorted[idx];
                for (t, &v) in terrain_type.iter_mut().zip(field.iter()) {
                    if v > threshold {
                        *t = TerrainType::Trees;
                    }
                }
            }
            Self::Urban {
                blocks,
                min_size_m,
                max_size_m,
            } => {
                let (lo, hi) = (min_size_m.max(1.0), max_size_m.max(min_size_m + 1.0));
                let cell = transform.cell_size_m();
                for _ in 0..blocks {
                    let cx = rng.random_range(0.0..extent_x);
                    let cy = rng.random_range(0.0..extent_y);
                    let half_x = rng.random_range(lo..hi) * 0.5;
                    let half_y = rng.random_range(lo..hi) * 0.5;
                    // Walk only the block's own cell range. Scanning the whole grid per
                    // block is O(blocks x cells) — 5 blocks on a 1000x1000 map is 5M
                    // visits to paint a few thousand. Bounds are clamped, and the RNG
                    // draws above are untouched, so the map is unchanged.
                    let ix0 = (((cx - half_x) / cell).floor().max(0.0)) as usize;
                    let iy0 = (((cy - half_y) / cell).floor().max(0.0)) as usize;
                    let ix1 = (((cx + half_x) / cell).ceil().max(0.0) as usize).min(width);
                    let iy1 = (((cy + half_y) / cell).ceil().max(0.0) as usize).min(height);
                    for iy in iy0..iy1 {
                        for ix in ix0..ix1 {
                            let p = transform.cell_center(ix, iy);
                            if (p.x - cx).abs() < half_x && (p.y - cy).abs() < half_y {
                                terrain_type[[iy, ix]] = TerrainType::Urban;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Evaluate a hill-sum field over every cell, in parallel.
///
/// This is the dominant cost of terrain generation: `O(cells x hills)`, and both the
/// relief and the woodland field pay it. The hills are *placed* sequentially from the
/// seeded RNG before this runs, and each cell writes only its own slot, so the result is
/// bit-identical to the serial version — the same reasoning that makes `los::viewshed`
/// parallel and deterministic.
fn hill_sum_field(hills: &[Hill], transform: &GridTransform, w: usize, h: usize) -> Array2<f32> {
    let mut out = Array2::<f32>::zeros((h, w));
    ndarray::Zip::indexed(&mut out).par_for_each(|(iy, ix), v| {
        let p = transform.cell_center(ix, iy);
        *v = hills.iter().map(|hill| hill.elevation_at(p)).sum();
    });
    out
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
