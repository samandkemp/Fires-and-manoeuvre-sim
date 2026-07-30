//! V5-V13 and V45 - line of sight, viewshed, mobility cost and slant range
//! (docs/DESIGN.md §1.4, §1.5, §9.1).

use glam::Vec2;
use ndarray::Array2;
use sim_core::los::*;
use sim_core::terrain::{TerrainGrid, TerrainParams, TerrainParamsTable, TerrainType};
use validation::{flat_with_patch, hills, params};

// V45 (docs/DESIGN.md §9.1): slant range is the true 3-D separation, reduces to the
// horizontal distance when the endpoints are at equal absolute height, and is
// symmetric. Overhead is not point blank.
#[test]
fn v45_slant_range() {
    let flat = flat_with_patch(64, 64, TerrainType::Open, 0..0, 0..0);
    let a = Vec2::new(100.0, 100.0);

    // Equal actor heights on flat ground ⇒ exactly the horizontal distance. This is
    // why adopting slant range leaves V14–V18 (all flat-range gates) untouched.
    let b = Vec2::new(500.0, 100.0);
    assert_eq!(slant_range(&flat, a, 2.0, b, 2.0), a.distance(b));

    // A drone directly overhead at altitude A is A away, not 0.
    assert!(
        (slant_range(&flat, a, 2.0, a, 802.0) - 800.0).abs() < 1e-3,
        "an overhead target must be its altitude away, not point blank"
    );

    // General case: √(horizontal² + Δheight²), and symmetric in the endpoints.
    let expected = (400.0f32 * 400.0 + 298.0 * 298.0).sqrt();
    let r = slant_range(&flat, a, 2.0, b, 300.0);
    assert!((r - expected).abs() < 1e-2, "got {r}, expected {expected}");
    assert_eq!(r, slant_range(&flat, b, 300.0, a, 2.0));

    // On relief the ground elevation difference counts too: the same horizontal
    // separation is a longer slant range across a valley than across the flat.
    let g = hills(7);
    let (p, q) = (Vec2::new(200.0, 200.0), Vec2::new(600.0, 200.0));
    let dz = g.sample_elevation(q) - g.sample_elevation(p);
    let expected = p.distance(q).hypot(dz);
    assert!((slant_range(&g, p, 2.0, q, 2.0) - expected).abs() < 1e-2);
    assert!(
        slant_range(&g, p, 2.0, q, 2.0) >= p.distance(q),
        "slant range is never shorter than the horizontal separation"
    );
}

// V5: on a flat open plane, everyone sees everyone; τ = 1.
#[test]
fn v5_flat_plane_mutual_visibility() {
    let g = flat_with_patch(64, 64, TerrainType::Open, 0..0, 0..0);
    let r = line_of_sight(&g, Vec2::new(55.0, 71.0), 2.0, Vec2::new(561.0, 402.0), 2.0);
    assert!(r.clear);
    assert_eq!(r.transmittance, 1.0);
    assert_eq!(r.canopy_length, 0.0);
    assert!(r.blocked_at.is_none());
    assert!(
        r.mask_height < 0.0,
        "flat ground must leave a clearance margin"
    );
}

// V6: a single wall hides exactly the similar-triangles shadow zone.
//
// Observer at x=25 (h=2), wall cells x∈[50,60) with top T=8, target at x=95.
// Grazing line over the near edge (s_w=25): required target height
//   h* = E_a + (T − E_a)·s_t/s_w − 0 = 2 + 6·(70)/25 = 18.8 m.
#[test]
fn v6_single_wall_closed_form() {
    let g = flat_with_patch(64, 8, TerrainType::Urban, 5..6, 0..8);
    let a = Vec2::new(25.0, 25.0);
    let b = Vec2::new(95.0, 25.0);
    let h_star = 2.0 + (8.0 - 2.0) * 70.0 / 25.0; // 18.8

    let above = line_of_sight(&g, a, 2.0, b, h_star + 0.2);
    assert!(
        above.clear,
        "target just above the shadow line must be visible"
    );

    let below = line_of_sight(&g, a, 2.0, b, h_star - 0.2);
    assert!(
        !below.clear,
        "target just below the shadow line must be hidden"
    );
    assert_eq!(below.transmittance, 0.0);
    // mask_height must report how far below the line the target sits.
    assert!(
        (below.mask_height - 0.2).abs() < 1e-2,
        "mask_height should be ~0.2 (got {})",
        below.mask_height
    );
    // The first block should be at the wall's near face (s = 25 from a).
    let s_block = below.blocked_at.expect("blocked_at must be set");
    assert!(
        (s_block - 25.0).abs() <= 5.0,
        "block near the wall face (got {s_block})"
    );
}

// V7: swapping endpoints preserves clear / τ / canopy length exactly.
#[test]
fn v7_symmetry() {
    let g = hills(17);
    let mut rng_s = 0x9E37u64;
    let mut next = move || {
        // Tiny LCG — test-local, deterministic, no rand dependency questions.
        rng_s = rng_s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((rng_s >> 33) as f32 / (u32::MAX >> 1) as f32).clamp(0.0, 1.0)
    };
    for _ in 0..50 {
        let a = Vec2::new(next() * 950.0, next() * 950.0);
        let b = Vec2::new(next() * 950.0, next() * 950.0);
        let fwd = line_of_sight(&g, a, 2.0, b, 2.0);
        let rev = line_of_sight(&g, b, 2.0, a, 2.0);
        assert_eq!(fwd.clear, rev.clear, "clear must be symmetric ({a} vs {b})");
        assert_eq!(
            fwd.transmittance, rev.transmittance,
            "τ must be symmetric exactly"
        );
        assert_eq!(fwd.canopy_length, rev.canopy_length);
    }
}

// V8: raising either endpoint never loses visibility; mask_height falls as the
// target rises; τ non-increasing in canopy width.
#[test]
fn v8_monotonicity() {
    let g = hills(23);
    let mut s = 0xC0FFEEu64;
    let mut next = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 33) as f32 / (u32::MAX >> 1) as f32).clamp(0.0, 1.0)
    };
    for _ in 0..30 {
        let a = Vec2::new(next() * 950.0, next() * 950.0);
        let b = Vec2::new(next() * 950.0, next() * 950.0);
        let base = line_of_sight(&g, a, 2.0, b, 2.0);
        let raised = line_of_sight(&g, a, 2.0, b, 7.0);
        if base.clear {
            assert!(raised.clear, "raising the target must never hide it");
        }
        assert!(
            raised.mask_height <= base.mask_height + 1e-3,
            "mask_height must fall as the target rises"
        );
        let obs_raised = line_of_sight(&g, a, 7.0, b, 2.0);
        if base.clear {
            assert!(
                obs_raised.clear,
                "raising the observer must never hide the target"
            );
        }
    }

    // Wider canopy ⇒ no more transmittance.
    let g1 = flat_with_patch(64, 8, TerrainType::Trees, 4..8, 0..8); // 40 m strip
    let g2 = flat_with_patch(64, 8, TerrainType::Trees, 4..12, 0..8); // 80 m strip
    let a = Vec2::new(15.0, 35.0);
    let b = Vec2::new(615.0, 35.0);
    let t1 = line_of_sight(&g1, a, 1.0, b, 1.0).transmittance;
    let t2 = line_of_sight(&g2, a, 1.0, b, 1.0).transmittance;
    assert!(t2 <= t1, "doubling the canopy must not increase τ");
}

// V9: the answer is invariant under translating / rotating the whole scenario.
#[test]
fn v9_rigid_motion_invariance() {
    // A flat map with one square hill of raised cells; move it around by whole
    // cells and move the endpoints with it.
    let make = |x0: usize, y0: usize| {
        let elev = Array2::from_shape_fn((48, 48), |(iy, ix)| {
            if (x0..x0 + 4).contains(&ix) && (y0..y0 + 4).contains(&iy) {
                30.0
            } else {
                0.0
            }
        });
        TerrainGrid::from_layers(
            10.0,
            elev,
            Array2::from_elem((48, 48), TerrainType::Open),
            &params(),
        )
    };

    let base = make(20, 20);
    let a = Vec2::new(105.0, 225.0);
    let b = Vec2::new(355.0, 225.0);
    let r0 = line_of_sight(&base, a, 2.0, b, 2.0);

    // Translation by (+5, +7) cells = (+50, +70) m.
    let shifted = make(25, 27);
    let d = Vec2::new(50.0, 70.0);
    let r1 = line_of_sight(&shifted, a + d, 2.0, b + d, 2.0);
    assert_eq!(r0.clear, r1.clear);
    assert!((r0.transmittance - r1.transmittance).abs() < 1e-5);
    assert!((r0.mask_height - r1.mask_height).abs() < 1e-3);

    // 90° rotation about the map centre: (x, y) → (extent − y, x). Cell centres of
    // the block (20..24, 20..24) map to the block (24..28, 20..24): centre x
    // 205..235 → x' = 480 − y ∈ 245..275 → ix' 24..27; y' = x ∈ 205..235 → iy'
    // 20..23.
    let extent = 480.0;
    let rot = |p: Vec2| Vec2::new(extent - p.y, p.x);
    let rotated = make(24, 20);
    let r2 = line_of_sight(&rotated, rot(a), 2.0, rot(b), 2.0);
    assert_eq!(r0.clear, r2.clear);
    assert!((r0.transmittance - r2.transmittance).abs() < 1e-5);
    assert!((r0.mask_height - r2.mask_height).abs() < 1e-3);
}

// V10: a canopy strip of width w crossed square-on gives τ = exp(−κ·w) exactly.
#[test]
fn v10_canopy_extinction_law() {
    let g = flat_with_patch(64, 8, TerrainType::Trees, 4..8, 0..8); // x ∈ [40, 80)
    let a = Vec2::new(5.0, 35.0);
    let b = Vec2::new(615.0, 35.0);
    let r = line_of_sight(&g, a, 1.0, b, 1.0);
    assert!(r.clear, "trees must not hard-block");
    let expected = (-0.08f32 * 40.0).exp();
    assert!(
        (r.transmittance - expected).abs() < 1e-3,
        "τ = {} but exp(−κw) = {expected}",
        r.transmittance
    );
    assert!(
        (r.canopy_length - 40.0).abs() < 0.5,
        "canopy length ≈ strip width"
    );

    // Above the canopy the strip is transparent.
    let high = line_of_sight(&g, a, 15.0, b, 15.0);
    assert_eq!(high.transmittance, 1.0);
    assert_eq!(high.canopy_length, 0.0);
}

// V11: the DDA agrees with a fine fixed-step reference sampler.
#[test]
fn v11_matches_fixed_step_oracle() {
    let g = hills(11);
    let mut s = 0xABCDu64;
    let mut next = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 33) as f32 / (u32::MAX >> 1) as f32).clamp(0.0, 1.0)
    };
    let mut checked = 0;
    for _ in 0..40 {
        let a = Vec2::new(next() * 950.0, next() * 950.0);
        let b = Vec2::new(next() * 950.0, next() * 950.0);
        let dda = line_of_sight(&g, a, 2.0, b, 2.0);
        let (oracle_clear, min_clearance) = oracle_clear(&g, a, 2.0, b, 2.0);
        // Near-tangent sightlines may legitimately differ between samplers; only
        // compare when the oracle's verdict is decisive.
        if min_clearance.abs() > 0.05 {
            assert_eq!(
                dda.clear, oracle_clear,
                "{a} → {b} (clearance {min_clearance})"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "oracle comparison must exercise a real sample"
    );
}

// V12: on a flat plane the viewshed is exactly the in-range disc of cells.
#[test]
fn v12_flat_viewshed_is_range_disc() {
    let g = flat_with_patch(64, 64, TerrainType::Open, 0..0, 0..0);
    let obs = Vec2::new(315.0, 315.0);
    let range = 200.0;
    let vs = viewshed(&g, obs, 2.0, 2.0, range);
    for iy in 0..64 {
        for ix in 0..64 {
            let in_range = g.transform().cell_center(ix, iy).distance(obs) <= range;
            let tau = vs[[iy, ix]];
            if in_range {
                assert_eq!(
                    tau, 1.0,
                    "in-range flat cell ({ix},{iy}) must be seen with τ=1"
                );
            } else {
                assert_eq!(tau, 0.0, "out-of-range cell ({ix},{iy}) must be 0");
            }
        }
    }
}

// V13: a high observer behind a wall casts exactly the closed-form shadow strip.
//
// Observer mast at x=25, h=20; wall column x∈[50,60), top 8; targets h=2 on flat
// ground. The descending grazing ray over the wall's *far* edge (s_w = 35) reaches
// 2 m at s* = 35·(20−2)/(20−8) = 52.5, i.e. x = 77.5: cells beyond the wall with
// centres x < 77.5 are hidden, x > 77.5 visible.
#[test]
fn v13_wall_shadow_strip_closed_form() {
    let g = flat_with_patch(64, 8, TerrainType::Urban, 5..6, 0..8);
    let obs = Vec2::new(25.0, 35.0);
    let vs = viewshed(&g, obs, 20.0, 2.0, 600.0);
    let iy = 3; // row of the observer (y = 35)
    for (ix, expect_visible) in [
        (4, true),
        (5, true),
        (6, false),
        (7, false),
        (8, true),
        (12, true),
    ] {
        let x = g.transform().cell_center(ix, iy).x;
        let tau = vs[[iy, ix]];
        assert_eq!(
            tau > 0.0,
            expect_visible,
            "cell ix={ix} (x={x}) expected visible={expect_visible}, τ={tau}"
        );
    }
}

// move_cost invariants: ≥ base distance cost, monotone in uphill grade, symmetric
// pair gives uphill > downhill, impassable propagates.
#[test]
fn move_cost_invariants() {
    let mut elev = Array2::zeros((4, 4));
    elev[[1, 2]] = 5.0; // one raised cell
    let g = TerrainGrid::from_layers(
        10.0,
        elev,
        Array2::from_elem((4, 4), TerrainType::Open),
        &params(),
    );

    let flat = g.move_cost((0, 0), (1, 0));
    assert!(
        (flat - 10.0).abs() < 1e-4,
        "flat open edge costs its length"
    );

    let up = g.move_cost((1, 1), (2, 1));
    let down = g.move_cost((2, 1), (1, 1));
    assert!(
        up > down,
        "uphill must cost more than the same edge downhill"
    );
    assert!(down > flat, "downhill still costs more than flat");

    let diag = g.move_cost((0, 0), (1, 1));
    assert!(diag > flat, "diagonal edge is longer");

    // Impassable neighbour → infinite edge.
    let mk = |mob| TerrainParams {
        feature_height_m: 0.0,
        extinction_per_m: 0.0,
        cover: 0.0,
        concealment: 0.0,
        mobility_cost: mob,
    };
    let blocked = TerrainParamsTable {
        open: mk(1.0),
        trees: mk(f32::INFINITY),
        urban: mk(1.5),
    };
    let g2 = flat_with_patch(4, 4, TerrainType::Trees, 1..2, 0..4);
    let g2 = TerrainGrid::from_layers(
        10.0,
        Array2::zeros((4, 4)),
        g2.terrain_type().clone(),
        &blocked,
    );
    assert!(g2.move_cost((0, 0), (1, 0)).is_infinite());
}

/// Reference implementation: march the sightline at quarter-cell steps, comparing
/// against bilinear ground. Returns (clear, minimum clearance seen).
fn oracle_clear(g: &TerrainGrid, a: Vec2, h_a: f32, b: Vec2, h_b: f32) -> (bool, f32) {
    let span = a.distance(b);
    let e0 = g.sample_elevation(a) + h_a;
    let e1 = g.sample_elevation(b) + h_b;
    let steps = (span / 2.5).ceil() as usize;
    let mut min_clear = f32::INFINITY;
    for i in 0..=steps {
        let s = span * i as f32 / steps as f32;
        let p = a + (b - a) * (s / span);
        let sight = e0 + (e1 - e0) * (s / span);
        min_clear = min_clear.min(sight - g.sample_elevation(p));
    }
    (min_clear >= 0.0, min_clear)
}
