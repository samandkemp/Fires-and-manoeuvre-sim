[Index](README.md) · [← §0 The strand index](00-strand-index.md) · [§2 Fires →](02-fires.md)

---

## 1. Terrain & LOS

Phase 1. Everything downstream (fires, sensing, movement) reads terrain and calls LOS,
so the conventions and the query contract here are the highest-blast-radius decisions in
the project.

### 1.1 Coordinate & unit conventions (forever)

- **World frame:** metres, `f32`. **X = east, Y = north, Z = up (elevation).** This is
  right-handed and agrees with Bevy 2D's Y-up, so the renderer never flips an axis.
  (`f32` resolves ~1 mm at 10 km — ample at tactical extents.)
- **Grid:** an `ndarray::Array2<T>` indexed `[iy, ix]` (row-major: row `iy` = northing,
  column `ix` = easting). **Origin at the south-west corner**; `ix` increases east,
  `iy` increases north. So a larger index is always further east / north — no inversion
  between grid, world, and screen.
- **Registration:** the stored value for cell `(ix, iy)` is the quantity at the **cell
  centre**, whose world position is
  `centre(ix, iy) = ((ix + 0.5)·s, (iy + 0.5)·s)` for cell size `s` metres.
  Continuous values between centres come from **bilinear interpolation**.
- **One transform.** `GridTransform` is the single place world↔cell conversion happens
  (`cell_center`, `world_to_cell`, `world_to_frac`). Any GIS raster (NW-origin,
  row-south) is flipped **once** at load, never in the models.
- **Cell size & extent** are scenario parameters. Default: `s = 10 m`, `1000 × 1000`
  cells (10 × 10 km, battlegroup scale — decision 2026-07-27).
- **Determinism.** Terrain and LOS are pure deterministic functions of their inputs.
  The only randomness is in *seeded generation* (procedural terrain), drawn from the
  caller's `SimRng`. Contract: same binary + same inputs → bit-identical output.
  Cross-platform bit-equality is not promised; float tests use explicit tolerances.

### 1.2 Height model — three distinct quantities, never conflated

- **Ground elevation** `z(x, y)` — bare earth (a "DTM"). Stored raster; bilinear between
  centres.
- **Feature height** `f(x, y) ≥ 0` — canopy / building height *above ground* (a "DSM"
  minus DTM). Per terrain type, from `terrain_types.toml`. The **blocking surface** is
  `z + f`.
- **Actor height** `h ≥ 0` — eye / mast / turret height of a unit or sensor above the
  ground it stands on. An LOS endpoint's absolute height is `z(endpoint) + h`.

Consequence: a unit *in* woods sits at `z + h` (under the canopy `z + f`), not on top of
it. Endpoints are excluded from their own cell's blocking.

### 1.3 Terrain grid & derived layers  *(plan step 1.1)*

`TerrainGrid` owns every layer so shape invariants live in one place:

- `elevation_m: Array2<f32>` — `z`, bare earth.
- `terrain_type: Array2<TerrainType>` — `Open | Trees | Urban` (extensible; `u8` repr).
- **Derived, precomputed at construction** from the per-type dials (cheap, pure):
  `feature_height_m` (`f`), `cover ∈ [0,1]`, `concealment ∈ [0,1]`,
  `mobility_cost ≥ 1` (`f32::INFINITY` = impassable).

Per-type dials (`scenarios/terrain_types.toml`, placeholder values):

| type  | feature_height_m | extinction κ (per m) | cover | concealment | mobility_cost |
|-------|------------------|----------------------|-------|-------------|---------------|
| Open  | 0                | 0                    | 0.0   | 0.0         | 1.0           |
| Trees | 12               | 0.08                 | 0.3   | 0.6         | 1.8           |
| Urban | 8                | 0 (hard-blocks)      | 0.7   | 0.5         | 1.5           |

**Terrain sources** behind a small seam so later ones are additive (D2):
analytic fixtures (flat via TOML; wall / ridge / hill built in tests — these double as
the LOS validation terrains) and seeded procedural terrain, all drawn from one RNG
stream in fixed order (relief → woods → urban): relief = Gaussian-hill sum (fBm later
if wanted); woodland = a second hill-sum field thresholded at the quantile giving the
requested `woods_fraction`; urban = seeded rectangular blocks (~200–500 m), overriding
woods. Real-DEM import is deliberately out of scope (decision: synthetic indefinitely)
but the seam keeps it a clean later addition.

**Composable recipes (2026-07-30).** `Flat` and `Hills` answer "give me a map"; they do
not answer "give me *this* map". `TerrainSource::Layers` takes a **recipe** — a base
surface plus ordered feature layers (`Ridge`, `Woodland`, `Urban`) — so a map can be
described: rolling relief, a ridge through the middle, light urban. Each layer is a small
deterministic op over `(elevation, terrain_type)` drawing from the one seeded stream, and
**the written order is part of the contract**: layers apply in sequence, so urban over
woodland leaves urban and the reverse does not. `TerrainSource::Preset` names common
recipes (`rolling_hills`, `wooded_hills`, `light_urban`, `dense_urban`, `mountain_pass`,
`flat_plain`), expanding to the same structure — sugar, not a second mechanism, so a
preset can always be copied out and adjusted.

`Flat` and `Hills` are deliberately **left as their own arms** rather than re-expressed as
recipes: they consume the RNG in an order that existing scenarios and gates depend on, and
rewriting them would silently change every seeded map. Gate: **V53**.

### 1.4 Line of sight  *(plan step 1.3 — the load-bearing primitive)*

**Query.** `los(a, h_a, b, h_b) -> LosResult`, world positions `a, b` with actor
heights `h_a, h_b`. Endpoint absolute heights `E_a = z(a) + h_a`, `E_b = z(b) + h_b`.

**Sightline as a height profile.** Parameterise the path by horizontal distance
`s ∈ [0, S]`, `S = ‖b − a‖`. The sightline height is a closure `hgt(s)`; for LOS it is
linear, `hgt(s) = E_a + (E_b − E_a)·s/S`. **Phase 2 reuses the identical traversal with
a parabolic `hgt(s)`** for ballistic crest-clearance — this closure is why LOS need not
be rewritten for indirect fire.

**Algorithm (D3):** Amanatides–Woo / DDA grid traversal from `a` to `b`, visiting exactly
the crossed cells. Sample the ground and blocking surfaces by **bilinear interpolation**
at the traversal points. Symmetric in `a ↔ b` by construction (a tested invariant).
A fixed-step sampler (step ≤ ½ cell) is kept as a **test-only oracle** to cross-check.

**Blocking semantics (D4):**
- **Ground mask (hard):** blocked where `hgt(s) < z(s)`.
- **Urban (hard):** blocked where the cell is Urban and `hgt(s) < z(s) + f_urban`.
- **Trees (soft):** accumulate canopy path length `L = Σ` (segment length inside Trees
  cells where `hgt(s) < z(s) + f_trees`); **transmittance** `τ = exp(−κ·L)`.
  `τ = 0` if any hard block occurs.

**Return type (D5) — rich, because the traversal computes it all anyway and sensing
needs more than a bool:**

```
LosResult {
    clear: bool,             // no hard (ground/urban) block
    transmittance: f32,      // τ ∈ [0,1]; exp(−κL); 0 if hard-blocked
    mask_height: f32,        // extra height at b needed to clear the worst hard mask
                             //   (negative ⇒ clearance margin) — defilade reasoning
    blocked_at: Option<f32>, // path distance s of the first hard block
    canopy_length: f32,      // Σ metres of sightline under canopy
}
```
plus `fn visible(...) -> bool` = `clear`. `mask_height` closed form: a hard mask of top
`T` at distance `s` is cleared iff `E_a + (E_b + Δ − E_a)·s/S ≥ T`, so the required
target-height increase is `Δ(s) = (T − E_a)·S/s − (E_b − E_a)`, and
`mask_height = max_s Δ(s)` over hard masks (−∞ → the reported margin if none).

### 1.5 Viewshed  *(plan step 1.4)*

One primitive, two uses: sensor-coverage display now, risk rasters for the DP layer
later.

**Brute force first (D6)** — run `los` from the observer to every in-range cell. Correct
by construction once `los` is validated, and it stays as the reference oracle any faster
sweep must match. It has not needed replacing: two optimisation passes made brute force
fast enough, both bit-identical in output, with V5–V13 and the V11 oracle as the check.

| Pass | Change | Measured |
|---|---|---|
| 2026-07-29 | LOS made allocation-free (thread-local scratch, incremental `mask_height`, cached endpoint elevation); `viewshed` parallel over cells (`ndarray` `Zip::par_for_each`, rayon) | 3 km viewshed 9.3 → 1.8 s |
| 2026-07-30 | Breakpoint **sort** removed: each axis's gridline crossings are already ascending in path distance, so the two streams merge in O(n) rather than `sort_unstable_by` | LOS 96.8 → 86.7 µs/query; 12 km viewshed 11.5 → 10.4 s |

Parallelism is deterministic: each cell writes its own slot and the scratch is
thread-local, so nothing depends on scheduling.

The second pass had been flagged as marginal and was worth **at least 10%** on long rays,
because a multi-kilometre ray carries ~2000 breakpoints. Short rays gain nothing (a 3 km
viewshed is unchanged). Later runs measured 80.6 µs, so run-to-run variance on this
machine is a few percent and 10% is the conservative read — the figure was measured, not
predicted, which was the point of taking the change at all.

**Mobility** is exposed to Phase 5 as `move_cost(from_cell, to_cell)` on cell **edges**,
not as a baked isotropic raster: slope direction matters, and uphill is penalised harder
than downhill. The constants are placeholder dials pending the Phase 5 movement TOML.

### 1.6 Validation matrix (the contract — each test names its analytical reference)

> **Where the gates live.** Every V-number below is enforced by a test in
> `crates/validation` (the sole exception is V52's zero-draw half, a unit test inside
> `sim_core` because it asserts a property of the RNG draw stream). The catalogue in
> `crates/validation/src/gates.rs` mirrors these tables machine-readably, and
> `crates/validation/tests/catalogue.rs` fails if the two ever disagree — a gate here with
> no test, or a V-numbered test not listed here, is a build failure rather than a silent
> gap. `cargo run -p validation --bin validation_report` prints the lot with its results.

| # | Property | Analytical reference | Step |
|---|----------|----------------------|------|
| V1 | world↔cell round-trip | `world_to_cell(cell_center(c)) = c` for all cells | 1.1 |
| V2 | bilinear exactness | sampling an affine field `z = ax+by+c` returns it **exactly** (bilinear reproduces planes) | 1.1 |
| V3 | derived layers well-formed | cover, concealment ∈ [0,1]; mobility ≥ 1; no NaN; deterministic from (type, dials) | 1.1 |
| V4 | generation determinism | same seed → bit-identical raster; different seed → different | 1.1 |
| V5 | flat plane visibility | any two actors with `h>0` on flat open ground: `clear`, `τ = 1` | 1.3 |
| V6 | single wall | hidden zone beyond the wall and `mask_height` match the similar-triangles closed form | 1.3 |
| V7 | LOS symmetry | `los(a,b).clear == los(b,a).clear`, equal `τ`, on random terrain | 1.3 |
| V8 | monotonicity | raising either endpoint never loses visibility; `mask_height` ↓ in target height; `τ` ↓ in canopy length | 1.3 |
| V9 | rigid-motion invariance | results invariant under whole-scenario translation and 90° rotation (catches axis-swap bugs) | 1.3 |
| V10 | canopy law | a uniform Trees strip of width `w` crossed square-on gives `τ = exp(−κw)` exactly | 1.3 |
| V11 | DDA vs oracle | DDA result agrees with the fixed-step sampler within a step-driven tolerance | 1.3 |
| V12 | flat viewshed = disc | on a flat plane, the viewshed is exactly the in-range cell set | 1.4 |
| V13 | ridge shadow | single infinite ridge: per-column shadow matches the V6 wall closed form | 1.4 |
| V53 | terrain recipes | a recipe + seed reproduces bit-identically; each layer meets its own invariant (woodland paints its `fraction`; a ridge lifts its crest line by `crest_m`); layer order is significant; presets differ as their names claim | 1.3 |

*(V53 is numbered out of sequence because composable recipes were added after Phase 9.
It is a terrain gate and belongs here, not with the air gates it was first written
beside.)*
