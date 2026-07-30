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
    // Reused breakpoint buffers so `line_of_sight` — called per cell in every viewshed —
    // does not allocate on the hot path. Three buffers: the two per-axis crossing streams
    // and the merged result (see `fill_breakpoints`).
    static SCRATCH: RefCell<Scratch> = const {
        RefCell::new(Scratch {
            xs: Vec::new(),
            ys: Vec::new(),
            merged: Vec::new(),
        })
    };
}

/// Per-thread reusable working buffers for the LOS traversal.
struct Scratch {
    xs: Vec<f32>,
    ys: Vec<f32>,
    merged: Vec<f32>,
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
        let scratch = &mut *buf.borrow_mut();
        fill_breakpoints(scratch, p0, p1, span, half_cell);
        let ss = &mut scratch.merged;
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

/// True 3-D range between two actors (`docs/DESIGN.md` §9.1): the horizontal separation
/// combined with the difference in absolute endpoint heights `z + h`.
///
/// This is the project's one range convention — detection cutoffs and weapon range gates
/// all use it. On flat ground with equal actor heights the height term vanishes and it
/// reduces exactly to `a.distance(b)`; for an airborne endpoint it is the difference
/// between "overhead" and "point blank".
#[must_use]
pub fn slant_range(terrain: &TerrainGrid, a: Vec2, h_a: f32, b: Vec2, h_b: f32) -> f32 {
    let horizontal = a.distance(b);
    let rise = (terrain.sample_elevation(b) + h_b) - (terrain.sample_elevation(a) + h_a);
    // hypot, not sqrt(x*x + y*y): it avoids overflow/underflow in the squares and is the
    // idiomatic Rust spelling of the Pythagorean length.
    horizontal.hypot(rise)
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

/// Fill `scratch.merged` with the ascending path distances (from `p0`, including `0` and
/// the span) at which the segment `p0 → p1` crosses any gridline of spacing `step` in x
/// or y.
///
/// The two per-axis crossing streams are each generated **already ascending in path
/// distance** (see [`axis_crossings`]), so they are combined by an O(n) merge rather than
/// an O(n log n) sort. On a multi-kilometre ray this buffer holds ~2000 breakpoints and
/// the sort was a measurable share of the query — this is the micro-optimisation
/// `docs/DESIGN.md` §1.5 flagged as outstanding. The output order is identical to the
/// sorted one, so results are bit-for-bit unchanged (V5–V13 are the check).
fn fill_breakpoints(scratch: &mut Scratch, p0: Vec2, p1: Vec2, span: f32, step: f32) {
    let Scratch { xs, ys, merged } = scratch;
    xs.clear();
    ys.clear();
    merged.clear();
    axis_crossings(xs, p0.x, p1.x, span, step);
    axis_crossings(ys, p0.y, p1.y, span, step);

    merged.push(0.0);
    let (mut i, mut j) = (0usize, 0usize);
    while i < xs.len() || j < ys.len() {
        // Take from whichever stream is next; `j` exhausted ⇒ take x, and vice versa.
        let take_x = j >= ys.len() || (i < xs.len() && xs[i] <= ys[j]);
        if take_x {
            merged.push(xs[i]);
            i += 1;
        } else {
            merged.push(ys[j]);
            j += 1;
        }
    }
    merged.push(span);
}

/// Push the path distances at which the segment crosses gridlines of spacing `step` on
/// one axis, **in ascending path-distance order**.
///
/// Walking `k` upward gives ascending `t` when the segment runs in +axis, and descending
/// `t` when it runs in −axis — so the loop walks `k` in whichever direction makes the
/// output ascending, which is what lets the caller merge instead of sort.
fn axis_crossings(out: &mut Vec<f32>, c0: f32, c1: f32, span: f32, step: f32) {
    let d = c1 - c0;
    if d.abs() <= 1e-9 {
        return;
    }
    let (lo, hi) = if c0 <= c1 { (c0, c1) } else { (c1, c0) };
    let (k_lo, k_hi) = ((lo / step).ceil(), (hi / step).floor());
    let mut k = if d > 0.0 { k_lo } else { k_hi };
    let dk = if d > 0.0 { 1.0 } else { -1.0 };
    while k >= k_lo && k <= k_hi {
        let t = (k * step - c0) / d;
        if t > 0.0 && t < 1.0 {
            out.push(t * span);
        }
        k += dk;
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
