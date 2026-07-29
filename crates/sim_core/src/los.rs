//! Line of sight over the terrain grid — the load-bearing primitive every fires and
//! sensing calculation reads. Specified in `docs/DESIGN.md` §1.4; validated by V5–V11.
//!
//! Semantics: the sightline runs between two actors at `z(endpoint) + h`. Bare ground
//! masks hard; Urban feature height masks hard; Trees attenuate — canopy path length
//! `L` gives transmittance `τ = exp(−Σ κ·L)`. Endpoint cells never contribute feature
//! blocking (an actor in woods looks out from under its own canopy).

use crate::terrain::{TerrainGrid, TerrainType};
use glam::Vec2;
use std::cell::RefCell;

thread_local! {
    // Reused breakpoint buffer so `line_of_sight` — called per cell in every viewshed —
    // does not allocate on the hot path.
    static SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Everything one LOS query learns. Cheap to compute — the traversal produces all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LosResult {
    /// True if no hard (ground / urban) mask blocks the sightline.
    pub clear: bool,
    /// Canopy transmittance `τ ∈ [0, 1]`: `exp(−Σ κ·L)`; forced to 0 when hard-blocked.
    pub transmittance: f32,
    /// Extra height the *target* (the `b` endpoint) would need to clear the worst hard
    /// mask; negative values are the clearance margin. Defilade reasoning in one number.
    pub mask_height: f32,
    /// Path distance from `a` of the first hard block, if any.
    pub blocked_at: Option<f32>,
    /// Metres of sightline running under attenuating canopy.
    pub canopy_length: f32,
}

/// Line of sight from actor at `a` (height `h_a` above ground) to actor at `b`
/// (height `h_b`). Symmetric: swapping the endpoints preserves `clear`,
/// `transmittance`, and `canopy_length` exactly (V7).
#[must_use]
pub fn line_of_sight(terrain: &TerrainGrid, a: Vec2, h_a: f32, b: Vec2, h_b: f32) -> LosResult {
    let span = a.distance(b);
    if span < 1e-6 {
        // Degenerate query: an actor always sees its own position.
        return LosResult {
            clear: true,
            transmittance: 1.0,
            mask_height: f32::NEG_INFINITY,
            blocked_at: None,
            canopy_length: 0.0,
        };
    }

    // Canonicalise the endpoint order so both query directions execute identical float
    // arithmetic — this is what makes the symmetry invariant exact rather than
    // approximate. (Lexicographic order on (x, y).)
    let swapped = (b.x, b.y) < (a.x, a.y);
    let (p0, h0, p1, h1) = if swapped {
        (b, h_b, a, h_a)
    } else {
        (a, h_a, b, h_b)
    };

    let e0 = terrain.sample_elevation(p0) + h0;
    let e1 = terrain.sample_elevation(p1) + h1;
    let dir = p1 - p0;

    // Cells whose *features* never block: the ones each endpoint stands in.
    let endpoint_cells = [
        terrain.transform().world_to_cell(p0),
        terrain.transform().world_to_cell(p1),
    ];

    // ---- Traversal --------------------------------------------------------------
    let hgt = |s: f32| e0 + (e1 - e0) * (s / span);
    let point = |s: f32| p0 + dir * (s / span);
    let half_cell = terrain.transform().cell_size_m() * 0.5;

    let mut hard_block = false;
    let mut first_block = f32::INFINITY;
    let mut last_block = f32::NEG_INFINITY;
    let mut optical_depth = 0.0f64; // f64 accumulator: many small κ·L terms
    let mut canopy_length = 0.0f32;
    let mut mask_height = f32::NEG_INFINITY; // running max of the mask-clearance lever

    // Segment boundaries at every half-cell gridline crossing, in a reused thread-local
    // buffer (no per-query heap allocation on this hot path). Terrain type changes at
    // cell boundaries and the bilinear ground patch at cell-centre lines, so within a
    // segment both are smooth; sampling ends + midpoint bounds the profile to half-cell
    // resolution (V11 cross-checks this against a fine fixed-step oracle).
    SCRATCH.with(|buf| {
        let ss = &mut *buf.borrow_mut();
        ss.clear();
        fill_breakpoints(ss, p0, p1, span, half_cell);
        ss.sort_unstable_by(f32::total_cmp);
        ss.dedup_by(|x, y| (*x - *y).abs() < 1e-6);

        // Ground elevation is shared between adjacent segments (s1 of one = s0 of next):
        // carry it, sampling only the new midpoint and far end each step.
        let mut z_s0 = e0 - h0; // z at the first breakpoint (s = 0 ⇒ p0)
        for win in ss.windows(2) {
            let (s0, s1) = (win[0], win[1]);
            let smid = 0.5 * (s0 + s1);
            let len = s1 - s0;
            let world_mid = point(smid);
            let cell = terrain.transform().world_to_cell(world_mid);
            let is_endpoint_cell =
                cell.is_some() && (cell == endpoint_cells[0] || cell == endpoint_cells[1]);

            let zm = terrain.sample_elevation(world_mid);
            let z_s1 = terrain.sample_elevation(point(s1));
            let (g0, gm, g1) = (hgt(s0), hgt(smid), hgt(s1));

            // Ground mask at the three sample points.
            record(
                s0,
                z_s0,
                g0,
                span,
                swapped,
                &mut hard_block,
                &mut first_block,
                &mut last_block,
                &mut mask_height,
            );
            record(
                smid,
                zm,
                gm,
                span,
                swapped,
                &mut hard_block,
                &mut first_block,
                &mut last_block,
                &mut mask_height,
            );
            record(
                s1,
                z_s1,
                g1,
                span,
                swapped,
                &mut hard_block,
                &mut first_block,
                &mut last_block,
                &mut mask_height,
            );

            // Feature effects come from the cell the segment runs through.
            if let (Some((ix, iy)), false) = (cell, is_endpoint_cell) {
                let ttype = terrain.terrain_type()[[iy, ix]];
                let f = terrain.feature_height()[[iy, ix]];
                let kappa = terrain.extinction()[[iy, ix]];
                match ttype {
                    TerrainType::Urban if f > 0.0 => {
                        record(
                            s0,
                            z_s0 + f,
                            g0,
                            span,
                            swapped,
                            &mut hard_block,
                            &mut first_block,
                            &mut last_block,
                            &mut mask_height,
                        );
                        record(
                            smid,
                            zm + f,
                            gm,
                            span,
                            swapped,
                            &mut hard_block,
                            &mut first_block,
                            &mut last_block,
                            &mut mask_height,
                        );
                        record(
                            s1,
                            z_s1 + f,
                            g1,
                            span,
                            swapped,
                            &mut hard_block,
                            &mut first_block,
                            &mut last_block,
                            &mut mask_height,
                        );
                    }
                    // Attenuating canopy: count the segment when the sightline runs under
                    // the canopy top at its midpoint.
                    _ if kappa > 0.0 && f > 0.0 && gm < zm + f => {
                        optical_depth += f64::from(kappa) * f64::from(len);
                        canopy_length += len;
                    }
                    _ => {}
                }
            }

            z_s0 = z_s1;
        }
    });

    let blocked_at = if hard_block {
        Some(if swapped {
            span - last_block
        } else {
            first_block
        })
    } else {
        None
    };

    LosResult {
        clear: !hard_block,
        #[allow(clippy::cast_possible_truncation)]
        transmittance: if hard_block {
            0.0
        } else {
            (-optical_depth).exp() as f32
        },
        mask_height,
        blocked_at,
        canopy_length,
    }
}

/// Boolean convenience: is there an unblocked hard sightline?
#[must_use]
pub fn visible(terrain: &TerrainGrid, a: Vec2, h_a: f32, b: Vec2, h_b: f32) -> bool {
    line_of_sight(terrain, a, h_a, b, h_b).clear
}

/// Brute-force viewshed (`docs/DESIGN.md` §1.5): the transmittance `τ` from an observer
/// at `observer` (height `h_obs`) to a target of height `h_tgt` at every cell centre
/// within `max_range_m`. `0.0` marks hard-blocked or out-of-range cells.
///
/// Correct by construction — one validated pairwise query per cell — and the reference
/// oracle for any faster sweep that may come later (V12–V13).
#[must_use]
pub fn viewshed(
    terrain: &TerrainGrid,
    observer: Vec2,
    h_obs: f32,
    h_tgt: f32,
    max_range_m: f32,
) -> ndarray::Array2<f32> {
    let (h, w) = (terrain.height(), terrain.width());
    let range_sq = max_range_m * max_range_m;
    let mut out = ndarray::Array2::<f32>::zeros((h, w));
    // Parallel over cells: each writes its own slot and the LOS scratch buffer is
    // thread-local, so the result is identical to the sequential version (V12–V13) —
    // determinism preserved, just faster on many cores.
    ndarray::Zip::indexed(&mut out).par_for_each(|(iy, ix), v| {
        let target = terrain.transform().cell_center(ix, iy);
        *v = if observer.distance_squared(target) > range_sq {
            0.0
        } else {
            let r = line_of_sight(terrain, observer, h_obs, target, h_tgt);
            if r.clear {
                r.transmittance
            } else {
                0.0
            }
        };
    });
    out
}

/// Fill `out` with path distances (from `p0`, plus `0` and the span) at which the segment
/// `p0 → p1` crosses any gridline of spacing `step` in x or y. Left unsorted; the caller
/// sorts and dedups the reused buffer.
fn fill_breakpoints(out: &mut Vec<f32>, p0: Vec2, p1: Vec2, span: f32, step: f32) {
    out.push(0.0);
    out.push(span);
    for (c0, c1) in [(p0.x, p1.x), (p0.y, p1.y)] {
        if (c1 - c0).abs() <= 1e-9 {
            continue;
        }
        let (lo, hi) = if c0 <= c1 { (c0, c1) } else { (c1, c0) };
        let mut k = (lo / step).ceil();
        while k * step <= hi {
            let t = (k * step - c0) / (c1 - c0);
            if t > 0.0 && t < 1.0 {
                out.push(t * span);
            }
            k += 1.0;
        }
    }
}

/// Fold one hard-surface sample at path distance `s` (surface top `top`, sightline height
/// `sight`) into the running block state and the `mask_height` clearance lever.
#[allow(clippy::too_many_arguments)]
fn record(
    s: f32,
    top: f32,
    sight: f32,
    span: f32,
    swapped: bool,
    hard_block: &mut bool,
    first_block: &mut f32,
    last_block: &mut f32,
    mask_height: &mut f32,
) {
    // Interior samples contribute to the target-clearance lever (endpoints excluded).
    if s > 0.0 && s < span {
        let s_from_a = if swapped { span - s } else { s };
        if s_from_a > 1e-6 {
            *mask_height = mask_height.max((top - sight) * (span / s_from_a));
        }
    }
    if sight < top {
        *hard_block = true;
        *first_block = first_block.min(s);
        *last_block = last_block.max(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{TerrainParams, TerrainParamsTable, TerrainSource};
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

    /// Flat, all-Open terrain with a rectangular patch of `patch` type.
    fn flat_with_patch(
        w: usize,
        h: usize,
        patch: TerrainType,
        x_cells: std::ops::Range<usize>,
        y_cells: std::ops::Range<usize>,
    ) -> TerrainGrid {
        let ttype = Array2::from_shape_fn((h, w), |(iy, ix)| {
            if x_cells.contains(&ix) && y_cells.contains(&iy) {
                patch
            } else {
                TerrainType::Open
            }
        });
        TerrainGrid::from_layers(10.0, Array2::zeros((h, w)), ttype, &params())
    }

    fn hills(seed: u64) -> TerrainGrid {
        // Bare relief (no woods/urban): the invariants under test are geometric.
        TerrainSource::Hills {
            count: 12,
            max_height_m: 90.0,
            base_radius_m: 150.0,
            woods_fraction: 0.0,
            urban_blocks: 0,
        }
        .build(10.0, 96, 96, seed, &params())
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
}
