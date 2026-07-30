//! V1-V4 - the terrain grid, its derived layers, and procedural generation
//! (docs/DESIGN.md §1.1, §1.3).

use glam::Vec2;
use ndarray::Array2;
use sim_core::terrain::*;

/// A params table with distinct, well-formed values for each type — enough to test
/// the derived-layer machinery without touching the TOML files.
fn test_params() -> TerrainParamsTable {
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

// V1: world↔cell round-trip.
#[test]
fn world_cell_roundtrip() {
    let tf = GridTransform::new(10.0, 5, 4);
    for iy in 0..tf.height() {
        for ix in 0..tf.width() {
            assert_eq!(tf.world_to_cell(tf.cell_center(ix, iy)), Some((ix, iy)));
        }
    }
    // Outside the grid → None.
    assert_eq!(tf.world_to_cell(Vec2::new(-1.0, 5.0)), None);
    assert_eq!(tf.world_to_cell(Vec2::new(5.0 * 10.0, 5.0)), None);
}

// V2: bilinear interpolation reproduces an affine surface exactly.
#[test]
fn bilinear_reproduces_planes_exactly() {
    let (w, h, s) = (8usize, 6usize, 10.0f32);
    let (a, b, c) = (0.7f32, -0.3f32, 5.0f32);
    let tf = GridTransform::new(s, w, h);
    let elevation = Array2::from_shape_fn((h, w), |(iy, ix)| {
        let p = tf.cell_center(ix, iy);
        a * p.x + b * p.y + c
    });
    let grid = TerrainGrid::from_layers(
        s,
        elevation,
        Array2::from_elem((h, w), TerrainType::Open),
        &test_params(),
    );

    // Sample strictly inside the cell-centre hull [(5,5), (75,55)] so no clamping.
    for &(x, y) in &[(15.0, 15.0), (37.3, 22.1), (50.0, 40.0), (12.5, 51.9)] {
        let got = grid.sample_elevation(Vec2::new(x, y));
        let want = a * x + b * y + c;
        assert!(
            (got - want).abs() < 1e-3,
            "at ({x},{y}): got {got}, want {want}"
        );
    }
}

// V3: derived layers are in range, finite, and deterministic from (type, dials).
#[test]
fn derived_layers_well_formed() {
    let (w, h) = (6usize, 6usize);
    let terrain_type = Array2::from_shape_fn((h, w), |(iy, ix)| match (ix + iy) % 3 {
        0 => TerrainType::Open,
        1 => TerrainType::Trees,
        _ => TerrainType::Urban,
    });
    let grid = TerrainGrid::from_layers(10.0, Array2::zeros((h, w)), terrain_type, &test_params());

    for &v in grid.cover().iter() {
        assert!((0.0..=1.0).contains(&v), "cover out of range: {v}");
    }
    for &v in grid.concealment().iter() {
        assert!((0.0..=1.0).contains(&v), "concealment out of range: {v}");
    }
    for &v in grid.mobility_cost().iter() {
        assert!(v >= 1.0 && !v.is_nan(), "mobility not >= 1: {v}");
    }
}

// V4: procedural generation is deterministic in the seed — all layers.
#[test]
fn procedural_terrain_is_deterministic() {
    let src = TerrainSource::Hills {
        count: 10,
        max_height_m: 100.0,
        base_radius_m: 200.0,
        woods_fraction: 0.25,
        urban_blocks: 2,
    };
    let g1 = src.build(10.0, 64, 64, 7, &test_params());
    let g2 = src.build(10.0, 64, 64, 7, &test_params());
    assert_eq!(
        g1.elevation(),
        g2.elevation(),
        "same seed must reproduce the raster"
    );
    assert_eq!(
        g1.terrain_type(),
        g2.terrain_type(),
        "same seed must reproduce the types"
    );

    let g3 = src.build(10.0, 64, 64, 8, &test_params());
    assert_ne!(
        g1.elevation(),
        g3.elevation(),
        "different seed should differ"
    );
}

// Type painting: the woods quantile paints roughly the requested fraction, and
// urban blocks exist when requested.
#[test]
fn procedural_type_painting_fractions() {
    let src = TerrainSource::Hills {
        count: 10,
        max_height_m: 100.0,
        base_radius_m: 200.0,
        woods_fraction: 0.3,
        urban_blocks: 2,
    };
    let g = src.build(10.0, 128, 128, 5, &test_params());
    let n = (128 * 128) as f32;
    let trees = g
        .terrain_type()
        .iter()
        .filter(|&&t| t == TerrainType::Trees)
        .count() as f32;
    let urban = g
        .terrain_type()
        .iter()
        .filter(|&&t| t == TerrainType::Urban)
        .count() as f32;
    // Urban overrides some woods cells, so allow slack below the quantile target.
    assert!(
        (0.18..=0.35).contains(&(trees / n)),
        "woods fraction {} should be near 0.3",
        trees / n
    );
    assert!(urban > 0.0, "urban blocks must paint at least one cell");
}

#[test]
fn flat_source_is_constant_and_open() {
    let src = TerrainSource::Flat { elevation_m: 42.0 };
    let g = src.build(10.0, 4, 3, 0, &test_params());
    assert_eq!(g.width(), 4);
    assert_eq!(g.height(), 3);
    assert!(g.elevation().iter().all(|&z| z == 42.0));
    assert!(g.terrain_type().iter().all(|&t| t == TerrainType::Open));
}
