//! Shared fixtures for the validation gates.
//!
//! Every gate needs terrain to run over, and before this crate existed each test module
//! carried its own copy of the same `params()` / `flat()` helpers — four copies of the
//! terrain dial table alone, which meant a dial could be changed in one gate's world and
//! not another's. They live here once.
//!
//! Fixtures are **deliberately explicit rather than loaded from `scenarios/`**: a gate
//! that checks a closed form must not change its answer because someone retuned a
//! placeholder dial. The one exception is [`scenario_params`], used by the gates that
//! test scenario loading itself.

use sim_core::scenario::load_terrain_params;
use sim_core::terrain::{
    GridTransform, TerrainGrid, TerrainParams, TerrainParamsTable, TerrainSource, TerrainType,
};
use std::path::{Path, PathBuf};

/// Cell size used by every fixture terrain, metres. Matches the project default so the
/// numbers in a failing assertion read the same as they do in a scenario.
pub const CELL_M: f32 = 10.0;

/// The canonical per-terrain-type dials the gates run against — the same values as
/// `scenarios/terrain_types.toml`, but pinned here so a gate's analytical reference
/// cannot be invalidated by retuning a placeholder.
#[must_use]
pub fn params() -> TerrainParamsTable {
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

/// A flat, all-`Open` grid — the fixture that isolates a model from terrain entirely.
#[must_use]
pub fn flat(w: usize, h: usize) -> TerrainGrid {
    TerrainGrid::from_layers(
        CELL_M,
        ndarray::Array2::zeros((h, w)),
        ndarray::Array2::from_elem((h, w), TerrainType::Open),
        &params(),
    )
}

/// Flat ground with a rectangular patch of `patch` terrain over the given cell ranges —
/// the wall/canopy fixture the LOS gates are built on.
#[must_use]
pub fn flat_with_patch(
    w: usize,
    h: usize,
    patch: TerrainType,
    x_cells: std::ops::Range<usize>,
    y_cells: std::ops::Range<usize>,
) -> TerrainGrid {
    let ttype = ndarray::Array2::from_shape_fn((h, w), |(iy, ix)| {
        if x_cells.contains(&ix) && y_cells.contains(&iy) {
            patch
        } else {
            TerrainType::Open
        }
    });
    TerrainGrid::from_layers(CELL_M, ndarray::Array2::zeros((h, w)), ttype, &params())
}

/// Bare seeded relief — no woods, no urban — so the invariant under test is purely
/// geometric.
#[must_use]
pub fn hills(seed: u64) -> TerrainGrid {
    TerrainSource::Hills {
        count: 12,
        max_height_m: 90.0,
        base_radius_m: 150.0,
        woods_fraction: 0.0,
        urban_blocks: 0,
    }
    .build(CELL_M, 96, 96, seed, &params())
}

/// A north–south ridge of height `crest` occupying cell columns `[x0, x1)` — the fixture
/// that makes terrain masking (and the AGL/AMSL distinction) observable.
#[must_use]
pub fn ridge(w: usize, h: usize, x0: usize, x1: usize, crest: f32) -> TerrainGrid {
    let elev =
        ndarray::Array2::from_shape_fn(
            (h, w),
            |(_, ix)| {
                if ix >= x0 && ix < x1 {
                    crest
                } else {
                    0.0
                }
            },
        );
    TerrainGrid::from_layers(
        CELL_M,
        elev,
        ndarray::Array2::from_elem((h, w), TerrainType::Open),
        &params(),
    )
}

/// A grid transform matching the fixture terrains, for gates that test the
/// world↔cell contract directly.
#[must_use]
pub fn transform(w: usize, h: usize) -> GridTransform {
    GridTransform::new(CELL_M, w, h)
}

/// Path to a file in the workspace `scenarios/` directory, resolved from this crate's
/// manifest dir so the gates do not depend on the working directory.
#[must_use]
pub fn scenario_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios")
        .join(name)
}

/// The workspace `scenarios/` directory itself.
#[must_use]
pub fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}

/// The *shipped* terrain dials, loaded from `scenarios/terrain_types.toml`. Used by the
/// gates that exercise scenario loading and the shipped scenarios themselves — unlike
/// [`params`], this is meant to change when the dials do.
#[must_use]
pub fn scenario_params() -> TerrainParamsTable {
    load_terrain_params(&scenario_path("terrain_types.toml")).expect("terrain dials should load")
}
