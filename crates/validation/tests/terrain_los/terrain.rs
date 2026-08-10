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

// ---- V53: composable terrain recipes (docs/DESIGN.md §1.3) -------------------------
// A recipe is only useful if it does what it says, reproducibly, and if the order of its
// layers is meaningful. These check each layer's own invariant plus the two properties
// that make a recipe a *description* of a map rather than a lucky seed.

/// Build a recipe on a 1 km square at 10 m cells.
fn build_recipe(recipe: TerrainRecipe, seed: u64) -> TerrainGrid {
    TerrainSource::Layers(recipe).build(10.0, 100, 100, seed, &validation::params())
}

fn type_fraction(g: &TerrainGrid, t: TerrainType) -> f32 {
    let n = g.terrain_type().iter().filter(|&&x| x == t).count();
    n as f32 / g.terrain_type().len() as f32
}

#[test]
fn v53_recipes_are_deterministic_and_layers_do_what_they_say() {
    let rolling = BaseRelief::Hills {
        count: 12,
        max_height_m: 80.0,
        base_radius_m: 250.0,
    };

    // Determinism: same recipe + seed -> bit-identical map; a different seed differs.
    let recipe = TerrainRecipe {
        base: rolling,
        apply: vec![
            TerrainLayer::Woodland {
                fraction: 0.3,
                patch_scale_m: 200.0,
            },
            TerrainLayer::Urban {
                blocks: 3,
                min_size_m: 100.0,
                max_size_m: 200.0,
            },
        ],
    };
    let a = build_recipe(recipe.clone(), 7);
    let b = build_recipe(recipe.clone(), 7);
    assert_eq!(a.elevation(), b.elevation(), "same seed -> same elevation");
    assert_eq!(
        a.terrain_type(),
        b.terrain_type(),
        "same seed -> same types"
    );
    let c = build_recipe(recipe.clone(), 8);
    assert_ne!(a.elevation(), c.elevation(), "a different seed must differ");

    // Woodland paints the fraction it was asked for (the quantile threshold makes this
    // exact up to ties in the field).
    for fraction in [0.1f32, 0.3, 0.55] {
        let g = build_recipe(
            TerrainRecipe {
                base: BaseRelief::Flat { elevation_m: 0.0 },
                apply: vec![TerrainLayer::Woodland {
                    fraction,
                    patch_scale_m: 150.0,
                }],
            },
            3,
        );
        let painted = type_fraction(&g, TerrainType::Trees);
        assert!(
            (painted - fraction).abs() < 0.02,
            "asked for {fraction} woodland, got {painted}"
        );
    }

    // A ridge raises the crest line by ~crest_m above the base, and the rise is local:
    // ground a long way from the line is untouched.
    let flat_base = TerrainRecipe {
        base: BaseRelief::Flat { elevation_m: 100.0 },
        apply: vec![TerrainLayer::Ridge {
            bearing_deg: 90.0, // runs north-south, so it crosses the map east-west
            crest_m: 200.0,
            width_m: 200.0,
            offset_m: 0.0,
        }],
    };
    let g = build_recipe(flat_base, 1);
    let peak = g.elevation().iter().copied().fold(f32::MIN, f32::max);
    assert!(
        (peak - 300.0).abs() < 1.0,
        "the crest should reach base + crest_m = 300 m, got {peak}"
    );
    // The far edge, several sigma off the line, is still at the base elevation.
    let edge = g.sample_elevation(Vec2::new(5.0, 500.0));
    assert!(
        (edge - 100.0).abs() < 1.0,
        "ground far from the ridge must be untouched, got {edge}"
    );

    // Layer *order* matters: urban laid over woodland survives, and the reverse buries it.
    let woods_then_urban = build_recipe(
        TerrainRecipe {
            base: BaseRelief::Flat { elevation_m: 0.0 },
            apply: vec![
                TerrainLayer::Woodland {
                    fraction: 0.9,
                    patch_scale_m: 200.0,
                },
                TerrainLayer::Urban {
                    blocks: 6,
                    min_size_m: 300.0,
                    max_size_m: 400.0,
                },
            ],
        },
        5,
    );
    assert!(
        type_fraction(&woods_then_urban, TerrainType::Urban) > 0.0,
        "urban applied last must survive the woodland beneath it"
    );

    // And a recipe with no layers leaves the base alone.
    let bare = build_recipe(
        TerrainRecipe {
            base: BaseRelief::Flat { elevation_m: 42.0 },
            apply: Vec::new(),
        },
        1,
    );
    assert!(bare.elevation().iter().all(|&z| z == 42.0));
    assert_eq!(type_fraction(&bare, TerrainType::Open), 1.0);
}

#[test]
fn v53_presets_expand_to_the_maps_they_name() {
    // At the scale the presets are tuned for: 10 km square. (On a 1 km map the 24 summed
    // hills of `rolling_hills` tower over everything and the names stop meaning anything.)
    let build = |p: TerrainPreset| {
        TerrainSource::Preset(p).build(50.0, 200, 200, 11, &validation::params())
    };

    // The flat plain really is flat and open — it is the fixture other gates lean on.
    let plain = build(TerrainPreset::FlatPlain);
    let (lo, hi) = plain
        .elevation()
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &z| (lo.min(z), hi.max(z)));
    assert_eq!((lo, hi), (0.0, 0.0), "a flat plain has no relief");
    assert_eq!(type_fraction(&plain, TerrainType::Open), 1.0);

    // Dense urban must actually be denser than light urban — the names have to mean
    // something relative to each other or the vocabulary is decoration.
    let light = type_fraction(&build(TerrainPreset::LightUrban), TerrainType::Urban);
    let dense = type_fraction(&build(TerrainPreset::DenseUrban), TerrainType::Urban);
    assert!(
        dense > light * 2.0,
        "dense urban ({dense:.3}) should be well beyond light urban ({light:.3})"
    );

    // Wooded hills carry more canopy than plain rolling hills.
    let rolling = type_fraction(&build(TerrainPreset::RollingHills), TerrainType::Trees);
    let wooded = type_fraction(&build(TerrainPreset::WoodedHills), TerrainType::Trees);
    assert!(
        wooded > rolling + 0.2,
        "wooded hills ({wooded:.2}) should be much greener than rolling ({rolling:.2})"
    );

    // The mountain pass has a *ridge*: a coherent linear rise, which `max - min` cannot
    // see (a broad ridge lifts the whole map, floor included). The invariant that does
    // detect it is the one the layer promises — ground on the crest line stands about
    // `crest_m` above ground far to either side of it.
    let crest_lift = |g: &TerrainGrid| {
        let centre = Vec2::splat(200.0 * 50.0 * 0.5);
        let dir = Vec2::from_angle(20f32.to_radians()); // MountainPass bearing
        let normal = Vec2::new(-dir.y, dir.x);
        let mean = |offset: f32| {
            let samples: Vec<f32> = (-8..=8)
                .map(|k| {
                    let p = centre + dir * (k as f32 * 400.0) + normal * offset;
                    g.sample_elevation(p)
                })
                .collect();
            samples.iter().sum::<f32>() / samples.len() as f32
        };
        // On the line, versus well clear of it on both sides.
        mean(0.0) - 0.5 * (mean(3500.0) + mean(-3500.0))
    };
    let pass = crest_lift(&build(TerrainPreset::MountainPass));
    let hills = crest_lift(&build(TerrainPreset::RollingHills));
    assert!(
        pass > 200.0,
        "the pass's crest line should stand ~320 m above the flanks, got {pass:.0} m"
    );
    assert!(
        hills.abs() < 100.0,
        "rolling hills have no ridge, so no systematic lift along that line ({hills:.0} m)"
    );
    assert!(
        pass > hills + 150.0,
        "the pass ({pass:.0} m) must be distinguishable from rolling hills ({hills:.0} m)"
    );
}
