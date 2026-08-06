# Design spec

The deep, quantitative spec: model formulations, equations, state machines, and the
validation reference for each subsystem. The README argues *why* each OR strand is the
right tool and states it in its own symbols; this file works each one out in full. A
section is written **before** its subsystem is implemented, and every model states the
analytical result or invariant its tests check against.

New to the codebase? Read `docs/HOW_IT_WORKS.md` first — it walks the same models in
plain terms, with worked numbers and the code path for each, and points back here for the
specification.

Sections are filled in roadmap order and all are now written. §7 (the simulation loop) was
deliberately written **last**: it specifies the order the models run in and the guarantees
that order buys, which only settled once four phases had each added to it.

---

## 0. The strand index

Each body of theory, the object it contributes, and where that object is worked out.
The README carries the rationale; this table is the map.

| Strand | Canonical object | Realised as | Section | Gates |
|---|---|---|---|---|
| Optimal control | `ẋ = f(x,u)`, `u ∈ U(x)` | turn-rate-limited flight; phase-integrated orbits | §9.2 | V46, V47 |
| Dynamic programming | `J*(x) = min_u [c(x,u) + J*(f(x,u))]` | least-risk pathing; Dijkstra as label-setting value iteration | §5 | V25–V27 |
| Stochastic processes | Poisson rates, Gaussian dispersion, Markov chains | detection `λ`; CEP and Carleton damage; suppression chain; time-to-kill | §§2–4, §9.4 | V14–V24, V28–V31, V48–V49 |
| Game theory | `v = max_x min_y xᵀAy` | zero-sum interdiction game by fictitious play | §6 | V32–V39 |
| Partial observability | belief `b_t(s)`, the posterior over enemy position given `z_{1:t}` | belief filter with negative information; EW on the rate | §8 | V40–V43 |
| Combinatorial optimisation | `max Σ payoff[i][j] x_ij` over an assignment | side-wide weapon–target allocation (Kuhn–Munkres) | §10.2 | V56 |
| Information-gain control | maximise `H(b) − E[H(b')]` over the available looks | belief-driven sensor tasking | §10.3 | V57 |

Two structural properties cut across all five and are treated as first-class:

- **Determinism.** Same binary, same `(scenario, seed)` → bit-identical output. Every
  phase added since has been *appended* to the loop and draws zero RNG when its inputs
  are empty, so each new subsystem reduces to an exact identity when switched off
  (§8 for EW, §9.6 for air). Cross-platform bit-equality is not chased; float tests use
  explicit tolerances.
- **Validated before optimised.** No model is made fast until it has a gate against a
  closed form. Where that meant keeping a slow reference (the brute-force viewshed, the
  fixed-step LOS oracle, the greedy allocator), the reference is kept.

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

---

## 2. Fires *(Phase 3 — after sensing)*

Direct fire (LOS-gated hit probability) and indirect fire (ballistic dispersion + area
effect + terrain interaction). Weapons are data-driven stat blocks
(`scenarios/weapons.toml`); a unit type may carry a `weapon` id. Every model has a
closed-form validation gate — this is the phase where "the maths is the product" is most
literally true.

> Each modelling choice below records the alternatives considered. All numbers are
> placeholder dials, not real munition performance data.

### 2.1 Weapon stat blocks (`scenarios/weapons.toml`)

Common: `class` (`direct` | `indirect`), `rof_rounds_per_min`, `max_range_m`,
`min_range_m` (indirect only, default 0).

- **Direct:** `dispersion_mrad` — 1σ angular aiming error (milliradians); the linear
  dispersion at range `r` is `σ(r) = dispersion_mrad · r / 1000` metres.
  `p_kill_given_hit` — lethality once a round strikes the target silhouette.
  `moving_target_penalty` — factor inflating σ against a moving target (dormant until
  movement, Phase 5).
- **Indirect:** `cep_m` — circular error probable at any range (a dial; range-dependent
  CEP is a documented later refinement). `lethal_radius_m` — Carleton damage scale `R_L`.

### 2.2 Direct-fire hit model

Requires LOS `clear` (Phase 1) and `r ≤ max_range`. The round's impact scatters as an
isotropic 2-D Gaussian about the aim point (= target centre) with σ = σ(r). The target
presents a rectangle of width `W` (deflection) and height `H` (elevation) — `W` from the
unit's `silhouette_width_m`, `H` from its `height_m`. Deflection and elevation errors are
independent, so

```
P_hit(r) = erf( W / (2·σ(r)·√2) ) · erf( H / (2·σ(r)·√2) )
P_kill    = P_hit · p_kill_given_hit · (1 − cover(cell(target)))
```

`erf` via the Abramowitz–Stegun 7.1.26 rational approximation (max error ~1.5e-7 — no
new dependency). *(Alternative considered: a single circular-target `P_hit =
1 − exp(−R²/2σ²)`; rejected because the rectangle silhouette gives distinct
deflection/elevation behaviour and a cleaner MC gate.)*

### 2.3 Indirect-fire dispersion & area effect

Impact point `b = aim + N(0, σ²I)` with `σ = cep_m / 1.1774` (the circular-Gaussian CEP
identity `CEP = σ·√(2 ln 2)`). Damage to a target at burst-to-target distance `ρ` is the
**Carleton function** `D(ρ) = exp(−ρ² / (2·R_L²))` — the standard OR incapacitation
kernel. For a single round aimed with offset `d` from a point target, the expected
damage marginalising over the Gaussian burst has a **closed form** (a Gaussian
convolution):

```
E[D](d) = R_L² / (σ² + R_L²) · exp( −d² / (2·(σ² + R_L²)) )
```

Delivered damage multiplies by `(1 − cover(cell(target)))` (urban/woods shielding —
reuses the terrain cover layer; no new dial). *(Alternatives: cookie-cutter lethality
disc — simpler but no smooth gate; elliptical range/deflection error (PER/PED) — more
realistic, needs the gun-target azimuth to orient the ellipse; deferred, circular CEP
first.)*

Crest clearance (a ridge blocking the ballistic arc) reuses the Phase 1 LOS traversal
with a **parabolic** height profile — the reason `line_of_sight` took a profile closure.
Wired as a later refinement; Phase 3 assumes a clear trajectory.

### 2.4 Fires in the sim loop

A `FireMission { shooter_unit, target }` executes on tick boundaries at the weapon's
rate of fire; each round draws from the seeded RNG (one draw block per round, fixed
order). Direct rounds sample hit/kill; indirect rounds sample a burst point and apply
`D·(1−cover)` damage. Units carry `strength ∈ [0,1]`, reduced by delivered damage;
`strength ≤ 0` ⇒ killed (removed from sensing/among live targets). This is the attrition
state Phase 4 validates against Lanchester.

### 2.5 Validation gates (V19–V24)

| # | Property | Reference |
|---|----------|-----------|
| V19 | direct-fire P_hit | MC fraction of impacts inside the `W×H` rectangle within CI of the erf-product |
| V20 | P_hit monotonicity | falls with range, rises with target size, falls with cover; = 0 when blocked or beyond max range |
| V21 | indirect CEP | empirical median miss distance of sampled bursts = `cep_m` within CI (Rayleigh) |
| V22 | area-damage closed form | MC mean of Carleton damage over sampled bursts = `E[D](d)` within CI, swept over `d` |
| V23 | damage monotonicity | `E[D]` falls with offset `d`, with cover, and rises with `lethal_radius` |
| V24 | fires determinism | same (scenario, seed, mission) → identical round outcomes and final strengths |

## 3. Sensing & detection *(Phase 2 — ordered before fires, decision 2026-07-27)*

The core interactive loop: place sensing assets, try to detect the enemy before being
detected. Detection is **mutual and asymmetric** — both sides' sensors run the same
machinery against the other side's units. Decisions (user, 2026-07-28): one generic
LOS sensor now with the schema shaped for acoustic and EO/IR later; glimpse-rate λ
model; dt = 1 s + 10 s epochs; static user-placed Red.

### 3.1 Stat blocks (all dials; `scenarios/sensors.toml`, `scenarios/units.toml`)

- **Sensor type:** `modality` (enum: `optical` now; `acoustic`, `eo_ir` later — each
  modality brings its own propagation term, which is why it is a tag, not a convention),
  `mount_height_m` (a mast or hovering UAS is just this dial), `max_range_m` (hard
  cutoff), `lambda0_per_s` (peak detection rate: against signature 1, τ = 1, zero
  concealment, at point-blank range), `range_half_m` + `range_exponent` (the falloff
  curve), optional field of regard (`for_width_deg`, sensor instance carries facing;
  default 360°).
- **Unit type:** `height_m` (target height for LOS), and `signature` as a **per-modality
  table** (`signature.optical = 0.6`) — acoustic/EO-IR add keys, not schema changes.
- **Sensor / unit instances** in the scenario: side (`blue`/`red`), type id, position,
  optional facing.

### 3.2 The glimpse-rate model

Detection of unit `u` by sensor `s` is a Poisson process with rate

```
λ(s, u) = λ0 · f(r) · sig_modality(u) · τ(s, u) · (1 − concealment(cell(u)))
f(r)    = 1 / (1 + (r / range_half)^range_exponent)
```

gated to zero when: LOS not `clear`, `r > max_range`, or `u` outside the field of
regard. `τ` and `clear` come from the Phase 1 LOS query (sensor `mount_height_m` and
unit `height_m` as the endpoint heights) — this is what the rich `LosResult` was built
for. `P(detect by t) = 1 − e^{−λt}`; per tick of length `dt`, `p = 1 − e^{−λ·dt}`.
Memoryless ⇒ results are **independent of the tick size** in distribution (V17 checks
the compounding identity exactly).

Detections were permanent in this phase, with track loss deferred to EW. **§10.1 replaced
that**: a track is held only while it is re-observed, and ages out `track_hold_s` after
its last observation. Acquisition is still the stochastic glimpse described here; only
maintenance was added. Each detection emits an event
`{time, sensor_id, unit_id, unit_pos}` into an append-only log (the POMDP-ready
observation channel, PLAN §5-S3) and marks the unit detected by the sensor's side.

### 3.3 The simulation loop (first appearance — DESIGN §7 made concrete)

`Sim::new(&scenario, seed)` resolves stat blocks and builds `SimState`; `Sim` owns the
`SimRng`. `step_one()` advances one tick `dt_s`; every `epoch_s` of sim time a (currently
empty) decision hook runs — fires/tasking phases fill it. Scenario `[sim]` block:
`dt_s = 1.0`, `epoch_s = 10.0`. Determinism: sensors and units iterate in fixed index
order; one RNG draw per live (sensor, undetected-unit) pair per tick; state carries no
hash-ordered containers.

### 3.4 Validation gates (V14–V18)

| # | Property | Reference |
|---|----------|-----------|
| V14 | detection-time distribution | constant-λ pair: empirical mean over seeded MC runs ≈ 1/λ within CI |
| V15 | closed form | MC frequency of detection by time t within binomial CI of 1 − e^{−λt} |
| V16 | rate structure | λ monotone non-increasing in range and concealment; exactly 0 when blocked, out of range, or outside field of regard; scales linearly in signature and τ |
| V17 | tick invariance | compounded per-tick survival Π(e^{−λ·dt}) equals e^{−λt} for dt ∈ {0.25, 0.5, 1, 2} (identity, float tolerance) |
| V18 | determinism | same (scenario, seed) → identical event log; different seed differs |

## 4. Suppression & attrition *(Phase 4)*

Decisions (user, 2026-07-29): units are **N sub-elements** attriting one at a time;
suppression is a **discrete Free/Suppressed/Pinned Markov chain** driven by near-miss
volume; fires validated against **Lanchester's square law**.

### 4.1 Units as N elements

A unit type carries `element_count` (default 1). `UnitState` tracks `elements`
(remaining) of `initial_elements`; `strength() = elements / initial` for display,
`alive() = elements > 0`. Attrition removes whole elements:

- **Direct round:** one round that passes `p_hit · p_kill_given_hit · (1 − cover)`
  removes **one** element.
- **Indirect round:** a burst delivers Carleton damage `D` to the unit; each surviving
  element is independently killed with probability `D · (1 − cover)` (a binomial draw) —
  so area fire attrits a group properly, and expected casualties `= elements · D · (1−cover)`.

Fire **volume scales with the shooter's live elements**: a unit fires
`round(rof · epoch/60) · elements` rounds/epoch (each element shoots). This is what makes
the aimed-fire duel obey Lanchester's square law.

### 4.2 Fire gating (revised)

- **Direct fire:** engage the nearest enemy in clear LOS and range — *no detection
  required* (you shoot what you can see).
- **Indirect fire:** engage the nearest **detected** enemy in range (you bombard where
  you have been cued). LOS not required (ballistic arc).

### 4.3 Suppression — the Markov chain

Per-unit state `S ∈ {Free, Suppressed, Pinned}`. Near-misses (rounds landing within
`suppression_radius_m` of the unit that do **not** kill it) push the state up; time
pushes it back down. Per near-miss, with probability `p_suppress` the state steps up one
level (Free→Suppressed→Pinned). Each tick, with rate `recover_per_s` the state steps
down one level (a memoryless recovery timer). Effects:

- **Free:** normal.
- **Suppressed:** outgoing fire effectiveness × `suppressed_fire_factor` (< 1) — degraded
  volume/accuracy; may still move.
- **Pinned:** cannot fire and cannot move (gates the Phase 5 movement layer).

Dials (`scenarios/*.toml`, placeholders): `suppression_radius_m`, `p_suppress`,
`recover_per_s`, `suppressed_fire_factor`.

### 4.4 Validation gates (V28–V31)

| # | Property | Reference |
|---|----------|-----------|
| V28 | chain stationary distribution | under a constant near-miss rate `λ_nm` and recovery `μ`, the long-run occupancy of {Free,Suppressed,Pinned} matches the analytic stationary distribution of the birth–death chain within CI |
| V29 | recovery time | with no incoming fire, mean time Pinned→Free = `2/recover_per_s` (two exponential steps), within CI |
| V30 | Lanchester square law | an aimed direct-fire duel (no terrain, no suppression) reproduces `α(A₀²−A²) = β(B₀²−B²)`: many-trial mean force curves match the ODE solution within CI |
| V31 | suppression gates fire | a Pinned unit emits no rounds; a Suppressed unit's expected output = `suppressed_fire_factor` × Free output |

## 5. Movement as DP *(Phase 5 — built out of roadmap order while Phase 4 awaits input)*

Least-risk pathing over the terrain grid: a mover chooses a route trading **mobility
cost** (time/effort) against **exposure risk**. The value function is exactly a
shortest-path / DP problem, so Dijkstra over the cell graph *is* the DP solution here
(no separate value-iteration needed until risk becomes time-varying in later phases).

### 5.1 Formulation

8-connected grid graph; each edge `from → to` costs

```
edge_cost = move_cost(from, to) + risk_weight · risk(to)
```

`move_cost` is the Phase 1 edge cost (mean mobility × slope factor × distance; ∞ =
impassable). `risk(cell)` is a caller-supplied exposure raster in `[0, 1]`; `risk_weight`
(metres of mobility-cost the mover will spend to avoid one unit of risk) tunes caution.
The least-cost path minimises total `Σ edge_cost` — an **additive** cost so the problem
is a clean shortest path. *(Alternative considered: multiplicative survival
`Π(1−p_death)` maximisation — richer but the log turns it additive anyway; additive with
a supplied risk raster is the smallest thing that gives the least-risk behaviour and a
clean Dijkstra gate. Documented in QUESTIONS §E.)*

Solved with Dijkstra (binary heap), skipping infinite (impassable) edges. Returns the
path and its total cost, or `None` if the goal is unreachable.

### 5.2 Risk raster

For the interactive demo, risk is **enemy observation coverage**: for each cell, the
detection rate a reference mover would suffer from the best-placed enemy sensor
(`max` over enemy sensors of `detection_rate` against a reference unit), normalised to
`[0, 1]`. This reuses the Phase 2 sensing model, so "least-risk path" literally means
"route that stays hardest to see" — the see-without-being-seen idea made navigable. The
path solver itself is agnostic: any `[0, 1]` raster works.

### 5.3 Validation gates (V25–V27)

| # | Property | Reference |
|---|----------|-----------|
| V25 | zero-risk = shortest path | with `risk_weight = 0` on uniform-mobility flat terrain, path cost equals the closed-form 8-connected distance `(max−min)+√2·min` scaled by cell mobility |
| V26 | risk avoidance monotone | raising `risk_weight` never increases total risk exposure along the optimum; a high-risk barrier gets routed around once the weight is high enough |
| V27 | optimality | Dijkstra cost matches an independent exhaustive/Bellman-Ford reference on a small grid; path is contiguous, in-bounds, endpoint-correct |

## 6. Game-theoretic layer *(Phase 6)*

Decisions (user, 2026-07-29): the first game is the **combined detect-then-engage
interdiction game**; solved by **fictitious play** (no LP dependency); strategies are
**Blue position vs Red route**. This is the capstone — it exercises terrain, LOS,
sensing, movement, fires, and suppression through one solved zero-sum game.

### 6.1 Movement in the sim (prerequisite)

Units gain a route and a speed. `UnitType.speed_m_s` (default 0 = static);
`UnitState` carries `route: Vec<Vec2>` + `route_idx`. Each tick, a live **unpinned**
unit advances `speed·dt` metres along its polyline (consuming multiple segments if a
tick's travel spans them); a **Pinned** unit does not move (wiring Phase 4 → Phase 5).
Reaching the last waypoint halts it. Detection and fires use the updated positions.
*Gates:* a unit on a straight route is at distance `speed·t` after `t` s; a pinned unit
does not advance.

### 6.2 The zero-sum solver — fictitious play

For a payoff matrix `A` (row = Blue/maximiser, col = Red/minimiser), fictitious play
alternates best responses to the opponent's empirical play:

```
each round:
  i* = argmaxᵢ Σⱼ A[i][j]·col_counts[j]     (Blue best-responds)
  j* = argminⱼ Σᵢ A[i][j]·row_counts[i]     (Red best-responds)
  row_counts[i*] += 1;  col_counts[j*] += 1
```

Time-average strategies `row_counts/T`, `col_counts/T` converge to a Nash equilibrium
and the value converges (Robinson 1951, for zero-sum). The value is bracketed by
`v_low = minⱼ (x·A[:,j])` (Blue's guarantee) and `v_high = maxᵢ (A[i,:]·y)` (Red's
guarantee); the gap `v_high − v_low → 0` measures convergence. Pure algorithm, no
dependency, and the convergence is itself an OR demonstration.

### 6.3 The interdiction payoff

- **Blue strategy:** a position `b` holding a sensor + a co-located **observed indirect**
  shooter (mortar). Detection (Phase 2) gates its fire (indirect ⇒ needs a detection).
- **Red strategy:** a route `r` (candidate paths across the map — some from the Phase 5
  least-risk pather at varying caution, some direct).
- **Payoff `A[b][r]` = expected Red attrition** (fraction of Red elements lost) when Red
  traverses `r` while Blue at `b` watches and bombards — estimated by a short headless
  Monte-Carlo battle averaged over seeds. Blue maximises attrition, Red minimises.
  Zero-sum in the attrition metric.

The matrix is built once (MC estimate per cell), then fictitious play solves it — so the
solver stays cheap even though payoff construction is the expensive part. Kept tractable
with small strategy sets (~6–8 each) and short battles, all in the Bevy-free
`experiments` crate; profile before growing.

### 6.4 Validation gates (V32–V39)

| # | Property | Reference |
|---|----------|-----------|
| V32 | matching pennies | FP value → 0, both strategies → (½, ½) |
| V33 | rock–paper–scissors | FP value → 0, both strategies → uniform |
| V34 | saddle point | a game with a pure equilibrium → that value, deterministic strategies |
| V35 | strict dominance | a strictly dominated strategy converges to ~0 weight |
| V36 | skew-symmetric | `A = −Aᵀ` ⇒ value 0 (fair game); value bracket closes |
| V37 | route-following | a unit on a straight route is at `speed·t` after `t` s (± one tick) |
| V38 | pinned halts | a Pinned unit does not advance along its route |
| V39 | interdiction sanity | a route outside every Blue position's view is "safe" (Red weights it, value falls); a Blue spot covering all routes raises the value |

## 7. The simulation loop

Written last, after four phases had each added to it. Every other section specifies a
*model*; this one specifies the **order those models run in and what that order
guarantees** — which is the thing most easily broken by accident and hardest to notice.

### 7.1 Two clocks, and why

The loop is hybrid continuous/discrete:

- **The tick** (`dt_s`, default 1 s) integrates what changes continuously — movement, and
  the moment-to-moment hazard of being seen.
- **The decision epoch** (`epoch_s`, default 10 s) is where *choices* are made: what is
  still being tracked, where to look, what to shoot.

The split is the optimal-control-plus-DP structure made concrete (README strands 1–2), and
it is load-bearing in both directions. Physically, fire missions are not re-planned sixty
times a minute. Computationally, it is what keeps the expensive decision layer — an
assignment solve and an information-gain search — off the hot path: §10.1 measured track
maintenance at ~2.3 ms per tick against ~0.23 ms at epoch cadence.

A tick may straddle an epoch boundary, or several. `step_one` advances the clock and then
resolves *every* boundary the new time has crossed, so `epoch_s` need not be a multiple of
`dt_s` and a coarse `dt_s` cannot silently skip a decision.

### 7.2 Phase order

The authoritative list. Phases 1–7 run every tick; phase 8 only on an epoch boundary.

| # | Phase | Draws RNG? | Spec |
|---|---|---|---|
| 1 | Ground movement along routes | no | §6.1 |
| 2 | Air movement, then carried-sensor sync | no | §9.2, §9.6 |
| 3 | Sensing vs enemy ground units | **yes** — one draw per eligible pair | §3.2 |
| 4 | Sensing vs enemy air | **yes** — one draw per eligible pair | §9.1 |
| 5 | Suppression recovery | **yes** — one draw per non-Free unit | §4.3 |
| 6 | Air-defence resolution | **yes** — per engagement due | §9.4 |
| 7 | Strike release | **yes** — burst point, damage rolls | §9.3 |
| 8a | Track maintenance | no | §10.1 |
| 8b | Sensor tasking | no | §10.3 |
| 8c | Fire allocation, then resolution | allocation no, rounds **yes** | §10.2, §2 |

Two orderings inside phase 8 are constraints, not preferences. **Tracks are maintained
before tasking**, because tasking reasons about what was *not* seen this epoch. **Tasking
precedes fires**, because indirect fire is gated on tracks — a sensor that loses contact
silences the guns behind it, and that must be visible in the same epoch.

Movement leads the tick so that sensing and fires act on current positions rather than
last tick's; a target that moved into cover this tick is in cover when it is looked at.

### 7.3 The determinism contract

**Same binary, same `(scenario, seed)` → bit-identical output.** Cross-platform
bit-equality is explicitly *not* promised (floats); float comparisons in gates use stated
tolerances.

Four structural rules make it hold, and each is enforced somewhere rather than trusted:

1. **One seeded stream.** All randomness comes from the `SimRng` the `Sim` owns
   (`ChaCha8Rng`, chosen because its stream is stable across `rand` versions — an archived
   seed must still reproduce after a routine dependency bump). No wall-clock, no thread
   RNG, no global state.
2. **Fixed iteration order.** Assets are visited by index, never by hash order; state
   carries no `HashMap` whose iteration reaches a result. Placement order is itself part
   of the contract (§10, `setup.rs`), so two runs agree on every index.
3. **Parallelism writes disjointly.** The rasters (viewshed, coverage, belief, risk) are
   computed with `rayon` via `ndarray::Zip`, where each cell writes its own slot and the
   LOS scratch is thread-local. No result depends on scheduling.
4. **New phases are appended and draw nothing when idle.** This is the rule that has let
   the loop grow from three phases to ten without invalidating older results.

### 7.4 The identity discipline

Rule 4 deserves stating as a design *method*, because it is how this project has added
every subsystem since Phase 3 without re-baselining what came before.

> A new subsystem must reduce to an **exact identity** when it has nothing to do — not an
> approximation, not "close enough". Switched off, the event log is bit-identical to the
> build before it existed.

Each such claim gets its own gate rather than being asserted:

| Subsystem | Identity when… | Gate |
|---|---|---|
| Electronic warfare | no jammers ⇒ every factor is exactly 1 | V40 |
| Air & counter-air | no air assets ⇒ phases 2, 4, 6, 7 draw zero RNG | V52 |
| Decision layer | one shooter, one reachable target ⇒ all rules agree | V58 |
| LOS memoisation | a cache hit is the value a miss would have computed | unit tests |

The payoff is concrete: adding drones did not move a single ground-scenario result, and
`sensor_tasking` could be added without touching the Phase 6 game — once it was defaulted
off, which V39 is what forced (§10.4).

The discipline also constrains *optimisation*, not just modelling. Both Phase-10 speed-ups
were verified by hashing a 4-scenario × 12-seed batch before and after and requiring the
digest to match. That is why the indirect damage factors are deliberately not
pre-multiplied into one term: float multiplication is not associative, and folding them
would shift a result by an ulp — enough to flip a knife-edge kill roll and silently
re-baseline V22 and V24.

### 7.5 What the loop does *not* do

Stated so the boundaries are visible rather than assumed:

- **No re-planning of movement.** Routes are fixed once given; a unit does not re-path
  around a threat that appears mid-run. Deferred as §10.5.
- **No intra-epoch reaction.** Both sides allocate against the board as it stood at the
  start of the epoch, so neither reacts to casualties the other has not taken yet. This is
  a deliberate simultaneity choice, not an oversight.
- **No variable time step.** A finer `dt_s` costs proportionally more; because detection
  is modelled as a rate rather than a per-tick probability (§3.2), it buys no accuracy in
  the detection statistics, which is exactly what V17 checks.

## 8. Electronic warfare & partial observability *(Phase 8)*

EW degrades sensing; with imperfect detection the observer reasons over a **belief** (a
probability distribution) rather than ground truth — the POMDP layer. EW is a clean
modifier on the sensing channel: with no jammers it is the identity, so EW-off reduces
bit-for-bit to §3 (V40).

### 8.1 EW — jamming as a sensing modifier

A jammer protects its own side's units by degrading the enemy's detection of them. A
jammer at `p` with `power ∈ [0,1]` and `radius` contributes a factor
`g = 1 − power·(1 − d/radius)` inside its radius (so `1−power` at the centre, `1` at the
edge); factors compose multiplicatively. The glimpse rate becomes
`λ_eff = λ · Π_j g_j(target)` over the target's own side's jammers — identity when there
are none. Data-driven: `[[side.jammers]] pos/power/radius_m` in the scenario.

### 8.2 Belief state — the POMDP layer

An **inference layer over the sim**, not sim state. `bayes_update(prior, likelihood)`
is the generic discrete posterior; a `SpatialBelief` holds a per-cell distribution over
enemy position with `update` (Bayes by an observation-likelihood raster), `predict` (a
diffusion motion model that raises entropy), `entropy`, and `most_likely_cell`. The key
observation model is **negative information**: `no_detection_likelihood` = per cell,
`P(no detection | enemy there)`, which reuses the sensing model *and EW* — cells the
sensor covers well get low likelihood (the enemy would have been seen), dead ground and
jammed cells get ~1. Multiplying an uninformative prior by the exposure-window
no-detection likelihood is "where an undetected enemy could be" — the app's belief
heatmap (green = cleared coverage, magenta = plausible hiding ground).

### 8.3 Validation gates (V40–V43)

| # | Property | Reference |
|---|----------|-----------|
| V40 | EW modifier | no jammers ⇒ factor exactly 1 and detection unchanged (EW-off = §3); a jammer cuts detection monotonically in power/proximity, sharply over a target |
| V41 | Tiger problem | `bayes_update` reproduces the exact posteriors (0.85 after one hear-left; 0.9698 after two; symmetric reversal) |
| V42 | belief well-formed | belief stays a normalised non-negative distribution; a peaked (detection) likelihood concentrates it and lowers entropy |
| V43 | negative information | repeatedly *not* detecting shifts belief out of coverage into dead ground; the motion model raises entropy |

## 9. Air: drones & counter-air *(Phase 9 — post-roadmap)*

Air assets are a **third class** alongside units and sensors: airframes that fly at a
chosen altitude, heading and speed along a flight path or a transit-then-orbit plan.
Red strike drones bomb Blue ground units; Blue defeats them with an **air-defence**
class whose engagement model — and therefore time-to-kill — varies by type. A drone may
instead (or also) carry a sensor, making it a mobile elevated observer.

The OR content is the **sensor-to-shooter timeline**: air defence can be self-cueing or
forced to rely on external cueing across a comms link with a configurable latency, so
raid leakage becomes a measurable function of cue delay, magazine depth, and engagement
channels. That falls straight out of the §3 sensing model.

Decisions (user, 2026-07-29): air is a new asset class (not a unit or sensor variant);
per-instance altitude with an AGL/AMSL reference; **slant range everywhere**; air
defence may carry an organic sensor with a per-instance self-cue switch and a comms
latency dial; strike targeting is **assigned only** (kill-chain logic is deferred as its
own piece); munitions are dials (`munitions` + `expendable`).

### 9.1 Altitude, actor height, and the slant-range convention

**Altitude is per instance**, with a reference frame, because the two behaviours differ
in exactly the way that matters — whether terrain can mask the airframe:

```
h(p) = altitude_m                                 (altitude_ref = agl)
h(p) = max(0, altitude_m − z(p))                  (altitude_ref = amsl)
```

`h` is precisely the **actor height** of §1.2, so LOS, viewshed, and sensing need no
change: `line_of_sight(terrain, a, h_a, b, h_b)` already takes arbitrary endpoint
heights. An AGL drone hugs the terrain and is never masked by the hill it overflies; an
AMSL drone cruises level and *is* masked by higher ground. The `max(0, ·)` clamp means an
AMSL altitude below the local ground is a drone on the deck — degenerate but well-defined,
never negative.

**Slant range replaces horizontal range** (a correction the air case forces, applied
everywhere for one consistent rule):

```
r_slant(a, h_a, b, h_b) = √( ‖b − a‖² + ((z(b) + h_b) − (z(a) + h_a))² )
```

used for the detection cutoff and falloff `f(r)` of §3.2 and for both weapon range gates
of §2. On flat ground with equal endpoint heights Δ = 0, so it reduces exactly to the old
horizontal range and the §3/§4 gates are unchanged by construction; on relief it differs
by the endpoint height difference. *(Alternative considered: slant only when an endpoint
is airborne — keeps ground-vs-ground bit-identical, but at the price of two range rules
that disagree by design. Rejected: one convention, documented.)*

**Terrain effects on an airborne target.** Concealment and cover are properties of the
cell a target *stands in*; an airborne target is not in it. So an air target contributes
`concealment = 0` to the §3.2 rate and `cover = 0` to damage. Canopy transmittance `τ` is
*not* waived — it is a property of the sightline, so a low drone seen through a belt of
woods is still attenuated exactly as §1.4 says.

### 9.2 Flight kinematics

State: `pos`, `altitude_m` + `altitude_ref`, `heading_deg`, `speed_m_s`. Pure and
RNG-free — flight is deterministic; all air stochasticity lives in detection and
engagement.

A **flight plan** is a waypoint list plus a terminal, which covers both requested
behaviours with one structure:

```
FlightPlan { waypoints: [Vec2], terminal: Hold | Orbit { radius_m, clockwise } }
```

"Fly this path" is `terminal = Hold`; "go here and orbit at radius R" is a single
waypoint with `terminal = Orbit`. The orbit centre is always the final waypoint.

- **Transit.** Advance `speed·dt` along the polyline (the §6.1 route logic). The desired
  heading is the bearing to the next waypoint; the actual heading turns toward it at up
  to `max_turn_rate_deg_s · dt`. A turn-rate limit implies a minimum turn radius
  `r_min = v / ω_max` (ω in rad/s) — the gate for V47.
- **Orbit.** On reaching the terminal waypoint the airframe captures the circle at its
  nearest point, then integrates the phase directly:
  `θ(t + dt) = θ(t) ± (v/R)·dt`, `pos = c + R·(cos θ, sin θ)`, heading = tangent.
  Integrating the phase rather than steering keeps the radius exact (no drift) and gives
  the closed-form lap time `T = 2πR/v`.
- **Endurance.** `endurance_s > 0` removes the airframe once its time aloft exceeds it.

### 9.3 Strike

A strike drone's aim point is its assigned target — a named unit or a fixed point — or,
if none was assigned, its **final waypoint**. (Autonomous target selection is a kill-chain
problem deferred deliberately; see §9.7.) On closing within `release_range_m` (slant) of
the aim point it releases one munition, which is **exactly the §2.3 indirect round**:
burst point `b = aim + N(0, σ²I)` with `σ = cep_m/1.1774`, Carleton damage
`D(ρ) = exp(−ρ²/2R_L²)`, delivered as `D·(1 − cover)`.

One generalisation of §2.3: an indirect round today damages only the unit it was aimed
at, but a strike on a *point* must damage whatever is near the burst. Damage is therefore
applied to **every live ground unit within `3·R_L` of the burst** (beyond 3 R_L the
Carleton kernel is < 1.2e-4 — a documented cutoff that keeps the sweep O(units)). Each
surviving element rolls independently, as §4.1; near-misses feed §4.3 suppression
unchanged.

`munitions` counts releases; `expendable` decides whether the airframe survives its
attack. Together they span the modern spectrum — a reusable guided-bomb carrier
(`munitions = 2, expendable = false`) and a one-way attack munition
(`munitions = 1, expendable = true`) are the same model with different dials.

### 9.4 Air defence — two engagement models, two closed forms

Two models, because **time-to-kill is distributed differently** in each — differing in
shape, not just in mean, so guns and missiles fail differently against a saturating raid.

**Gun / CIWS — a Poisson kill process.** While the target sits in the envelope, kills
arrive at rate `λ_k`; per tick `p = 1 − e^{−λ_k·dt}`. This is structurally identical to
the §3.2 glimpse model, so it inherits its tick-size invariance and its validation
machinery:

```
TTK ~ Exp(λ_k)          E[TTK] = 1/λ_k          P(kill by t) = 1 − e^{−λ_k·t}
```

**Missile — discrete shoot-look-shoot.** A launch takes `t_f = r_slant / missile_speed`
to arrive, then resolves as a Bernoulli trial with single-shot kill probability `p`; a
miss is followed by `t_r` reload before the next launch. Shots-to-kill is Geometric(p),
and the time to the N-th arrival is `N·t_f + (N−1)·t_r`, so

```
E[shots] = 1/p          E[TTK] = t_f/p + (1/p − 1)·t_r
```

*(Alternative considered for the missile: model interception kinematically, with the
missile as a pursuing body and the kill depending on closing geometry. Rejected for v1 —
the interesting OR variable here is delay and magazine, not endgame guidance; `p` and
`t_f` are the dials that carry the behaviour.)*

**Envelope.** An engagement requires the target inside `[min_range_m, max_range_m]`
(slant) *and* `[min_alt_m, max_alt_m]` — the altitude band is what separates a low-tier
CIWS from a high-tier SAM — plus LOS if `requires_los`, a free engagement channel, and
magazine remaining. `channels` (simultaneous engagements) is the saturation lever a raid
plays against.

### 9.5 The cueing timeline

A battery acts on whichever route to the track reaches it first — its own radar, or the
network:

```
actionable_at = min( own_sensor_seen,                  // organic: no comms hop
                     first_detected + cue_latency_s )  // handed over the net
              + reaction_time_s
```

`own_sensor_seen` is when **this battery's** organic sensor first saw the target, and is
unavailable if it has no sensor, its per-instance `self_cue` switch is off, or it simply
has not seen the target yet. Turning `self_cue` off therefore forces the asset onto the
external cueing chain and makes it pay `cue_latency_s` — the comms Tx/Rx lever.

Taking the **minimum** is what makes this exact rather than approximate. Every airframe
records when each sensor first saw it (`AirState.seen_by`), so a self-cueing battery whose
radar acquires the target *after* someone else's sensor detected it still engages off its
own radar instead of waiting out a comms hop it never needed. *(An earlier version keyed
"self-cued" off whoever detected first, which got that case wrong; the per-sensor record
replaced it. Consequence: the air detection loop runs each sensor's glimpse process until
**that sensor** has seen the target, rather than stopping at the first global detection.)*

This yields the phase's headline closed form, and it turns on **what the clock starts
on**: the cueing chain begins at *detection*, not at envelope entry. Let a drone be detected
`D` seconds before it enters the envelope (its **warning lead**) and spend `W` seconds
inside the envelope before reaching its release point. The battery is actionable from
`t_entry − D + L + R`, so the effective engagement window is

```
W_eff = max(0, W − max(0, L + R − D))
```

The delay costs nothing until `L + R` outruns `D`: a cue that has already aged through
the network while the drone was still inbound arrives ready. Consequently the **critical
latency**, beyond which every drone leaks however lethal the battery is, is

```
L* = W + D − R
```

and early warning raises it one second per second — early-warning range and comms latency
trade *directly* against each other. For a gun `P(leak) = exp(−λ_k · W_eff)`; for a
missile with `K = ⌊(W_eff − t_f)/(t_f + t_r)⌋ + 1` shot opportunities (0 if `W_eff < t_f`),
`P(leak) = (1 − p)^K`.

*(The `D = 0` case — acquired exactly as it enters the envelope — gives the simpler
`W_eff = W − L − R` and `L* = W − R`. That special case is what the `air_raid` experiment
isolates in sweep 1a by setting the radar's range equal to the gun's; sweep 1b sweeps `D`
deliberately. The general form above was corrected after the first sweep read zero
leakage everywhere: with a 5 km radar and a 2.5 km envelope the transit time absorbed the
entire cue delay, which is the model behaving correctly and the earlier formula being an
unstated special case.)*

### 9.6 Determinism & the air-off identity

Air adds phases to the loop. They are **appended, and draw zero RNG values when the air
and air-defence lists are empty**, so a drone-free scenario reproduces the pre-air event
log bit-for-bit — the identity discipline §7.4 states in general and EW follows in §8
(V40). The full phase order lives in **§7.2**; the air phases are 2, 4, 6 and 7.

A recce drone's sensor needs no phase of its own: **every sensor lives in one list**, and
a carried one reports the position, height and facing of the airframe carrying it
(`Sim::sensor_view`). For an uncarried sensor that resolves to exactly its own position
and `mount_height_m`, so step 3 is unchanged — same draws, same order — whenever there is
no air. Steps 4, 6 and 7 iterate empty lists in that case and draw nothing at all.

A carried sensor's public `pos`/`facing_deg` are also **written back from the airframe each
tick**, immediately after air movement. The airframe stays the source of truth and
`sensor_view` is still the accessor that knows about altitude, but leaving the public
fields frozen at the placement point made any consumer outside the detection loop — the
app's coverage and belief overlays, the `duel_probe` diagnostic — plot a recce drone's
sensor at its take-off position and ground mount height. `Sim::sensor_active` is the
matching gate: a carried sensor dies with its airframe, so a shot-down drone must drop out
of coverage rasters as well as out of the detection loop.

Ground fires cannot accidentally engage air: target selection iterates the *unit* list, so
the separation is structural rather than a gate that could be forgotten.
`WeaponType.engages_air` (default false) exists as the opt-in seam for a future dual-role
gun; it changes nothing today.

### 9.7 Deliberate limitations (v1)

Stated rather than hidden, each a clean later addition and none a refactor:

- **Air-defence sites are not attritable** — they are standalone placed assets, so a
  strike drone cannot yet conduct SEAD. `AirDefenceState.carrier` exists unused so that
  mounting a launcher on a unit later is a small change.
- **No autonomous target selection.** Strike targets are assigned (§9.3); a drone will
  not opportunistically attack what its own sensor finds.
- **No air-to-air.** Drones do not engage other drones.
- **Acoustic detection of drones** is the natural modality and remains unimplemented —
  §3.1's `Modality` tag is the seam.
- ~~**Detection of air is permanent.**~~ **Lifted by §10.1.** Air tracks now decay like
  ground tracks, and a lapsed one clears the cueing record so reacquisition restarts the
  §9.5 timeline. The `seen_by` design anticipated this, and the change was contained as
  predicted.

### 9.8 Validation gates (V44–V52)

| # | Property | Reference |
|---|----------|-----------|
| V44 | altitude & masking | an AMSL drone below a ridge crest is masked from a ground sensor while the same drone at AGL is not; visibility is monotone in altitude (extends V8) |
| V45 | slant range | equals `√(horizontal² + Δz²)` exactly; a drone at altitude `A` directly overhead has range `A`, not 0; reduces to horizontal range when Δz = 0 (so V14–V18 stand unchanged) |
| V46 | orbit kinematics | orbiting at radius `R` and speed `v` holds the radius to within ε and closes a lap in `2πR/v` (± one tick) |
| V47 | transit & turn rate | straight-leg travel = `speed·t` (mirrors V37); under a turn-rate limit the achieved turn radius ≥ `v/ω_max` |
| V48 | gun time-to-kill | MC mean TTK = `1/λ_k` and `P(kill by t) = 1 − e^{−λ_k t}` within binomial CI — the §9.4 exponential law, gated as V14/V15 |
| V49 | missile time-to-kill | per-shot kill fraction = `ssk_p` within binomial CI; `E[shots] = 1/p` (geometric); `E[TTK] = t_f/p + (1/p − 1)t_r` |
| V50 | cue latency & leakage | leakage rises monotonically in `cue_latency_s`, matches `exp(−λ_k·W_eff)`, and reaches 1 above the critical latency; warning lead `D` raises `L* = W + D − R` one second per second, and `D = 0` reproduces `W − L − R` |
| V51 | envelope & magazine gating | exactly zero engagements outside the slant-range band, outside the altitude band, without LOS when `requires_los`, without a cue when `self_cue` is off, or with an empty magazine; concurrent engagements never exceed `channels` |
| V52 | air-off identity | no air and no air defence ⇒ event log bit-identical to the pre-air build; with air, same `(scenario, seed)` reproduces exactly |

## 10. The decision layer *(Phase 10 — complete)*

Phases 1–9 modelled everything except anyone *deciding* anything. Fires picked the
nearest enemy, routes were drawn by hand, sensors stared where they were placed, and the
belief filter of §8.2 was computed for a UI overlay that no sim code read. This phase
closes the loop **sensing → belief → decision → action**: tracks now decay (§10.1), fire
is allocated side-wide (§10.2), and sensors point themselves by belief (§10.3).

The decision epoch of §3.3 was designed as the hook for exactly this. It currently calls
only `resolve_fires()`.

Decisions (user, 2026-07-31): tracks decay by **hold time**; fire allocation is
**side-wide and optimal**, with a greedy allocator kept alongside so the optimality gap
is measured rather than assumed; target value is an **optional dial** over a derived
default; sensor tasking maximises **expected information gain**.

### 10.1 Track lifecycle *(landed)*

§3.2 made detection permanent, with track loss deferred to EW. That deferral turned out
to have teeth: a unit once seen stays seen forever, so **jamming a tracked unit does
nothing at all**. EW could prevent a track but never break one, which is half the model
missing rather than a simplification.

Detection is now derived from a last-observation time. `UnitState` and `AirState` carry
`last_seen_s: Option<f64>`, and at each epoch

```
detected  ⟺  now − last_seen_s  <  track_hold_s
```

`detected` stays the field everything else reads, so indirect-fire gating (§4.2) and the
§9.5 cueing timeline needed no change. `track_hold_s` is a scenario dial. Air keeps its
per-sensor `seen_by` record as well, because §9.5 needs to know *which* battery saw the
target.

A lapsed air track clears the whole cueing record — `detected_at_s`, `detected_by` and
`seen_by` — not just the flag. Otherwise reacquisition would find a stale `detected_at_s`
already aged past `cue_latency_s + reaction_time_s` and a battery would fire the instant
the target reappeared, skipping the §9.5 timeline the scenario exists to exercise.

**Maintenance runs at the decision epoch, not the tick.** The glimpse loop skips
already-detected targets, so refreshing a track means looking again — measured at
4 sensors × 6 units × 97 µs ≈ 2.3 ms/tick, up to 20× the whole tick budget. At a 10 s
epoch that amortises to 0.23 ms/tick. The cadence is also right on its own terms: tracks
decay over tens of seconds, and maintaining one is a decision-layer concern.

**Maintenance is deterministic, not a fresh glimpse.** Acquisition stays stochastic;
keeping eyes on something already found is not a coin flip. A track refreshes when the
sensor's *effective* rate `λ_eff` — the §8.1 jammed rate, with concealment, range and
canopy folded in — clears a `track_maintain_p` threshold:

```
refresh  ⟺  1 − e^{−λ_eff·Δt_epoch}  ≥  track_maintain_p
```

Using the effective rate rather than bare geometry is what lets EW break a track:
a jammer that drives `λ_eff` below the threshold ages the track out even with clean LOS.
A pure "can it still be seen?" test would have re-opened the exact gap this closes.
Drawing nothing also leaves the per-tick RNG stream unperturbed.

### 10.2 Fire allocation *(landed)*

Replaces the nearest-enemy rule with a side-wide assignment, solved once per epoch per
side before anyone shoots. For shooter `i` and slot `k` of target `j`,

```
payoff[i][(j,k)] = q(i,j) · value(j) · (1 − q̄(j))^k
```

`q(i,j)` is the **fraction of the target destroyed this epoch**, from the existing fires
model — `direct_p_hit` or `expected_area_damage`, times cover, suppression factor and
round count, exactly as a round resolves — clamped to `[0,1]`. Ineligible pairings (out
of range, no LOS, undetected for indirect) are forbidden outright.

`value(j)` is `elements × per_element`, where `per_element` is the optional `value` dial
on the stat block, or `1 + threat/threat_max` when absent, with
`threat = rof × p_kill_given_hit × max_range`. So an unscored stat block still ranks
sensibly — a unit is worth its size, doubled if it is the most dangerous thing on the
field — and doctrine ("kill the radar first") can be stated when wanted. Per *element*,
so a half-destroyed unit is correctly worth less.

**Slots and the discount.** A target with `E` elements offers `min(E, cap)` slots, and
slot `k` is discounted by `(1 − q̄)^k` for a representative `q̄`: the (k+1)-th shooter
only helps if the `k` before it all failed. This is the standard weapon–target-assignment
decomposition and is exact when the shooters on a target are alike. It turns diminishing
returns into extra columns, keeping the problem a plain linear assignment rather than a
submodular one.

Solved by Hungarian (Kuhn–Munkres) over shooters × slots, with greedy and `independent`
(the old per-shooter rule) alongside. `[sim] allocation` chooses.

**Forbidden pairings are scored zero, not `−∞`.** Kuhn–Munkres produces a *perfect*
matching, while what is wanted is a maximum-weight matching that may leave a shooter
idle. With non-negative payoffs and rows ≤ columns, any partial matching extends to a
perfect one using only zero-weight cells without changing its total — so the two optima
coincide, and assignments landing on a forbidden or worthless cell are simply dropped
afterwards. A large negative sentinel would have been worse than wrong: `1e18 + 10.0`
is `1e18` in `f64`, so every matching with the same number of forbidden cells would have
scored identically.

#### Measured: coordination pays, optimality does not *(2026-08-04)*

`experiments/allocation_gap` runs each scenario under all three rules on the same seeds,
and compares **paired** — the per-seed difference cancels the map and the dice, leaving
only the effect of the rule. On `scenarios/fire_allocation.toml` (four shooters that can
all reach all four targets), 500 seeds:

| Rule | Time to destroy Red | vs `independent`, paired | Targets engaged per epoch |
|---|---|---|---|
| `independent` (the old rule) | 75.1 ± 0.4 s | baseline | 1.00 |
| `greedy` | 63.8 ± 0.4 s | **−11.30 ± 0.47 s** (significant) | 3.02 |
| `optimal` | 63.9 ± 0.4 s | **−11.18 ± 0.49 s** (significant) | 3.02 |

**Coordinating is worth ~15%**, unambiguously, and the mechanism is visible in the spread
column: the old rule sent every gun at the nearest target while three others stood
untouched.

**Optimality is worth nothing measurable.** Greedy and Hungarian differ by 0.12 s against
a standard error of ~0.5, and produce *identical* outcomes on the large majority of seeds
(they diverge at all on well under a fifth). On instances this size — a handful of
shooters against a handful of similar targets — greedy's myopia costs it essentially
nothing, because there is rarely a case where taking the locally best pairing forecloses
a better global one. The optimal solver is kept because it is the reference V56 checks
against and because the gap should be re-measured as scenarios grow, not because it is
currently earning its extra complexity.

> **A methodological correction worth recording.** This section previously claimed greedy
> *beat* the optimal solver "consistently and outside the noise". It did not. That came
> from comparing two unpaired means (30 and 200 seeds) whose difference sat inside the
> sampling error, with no standard error reported to make that visible. A paired test over
> 500 seeds gives a mean difference of −0.12 s with SE 0.22 (t = −0.55): no effect.
> `allocation_gap` now reports a standard error on every figure and pairs every
> comparison, because a bare mean is precisely what invited the wrong conclusion.

On the other shipped scenarios the difference is exactly zero on every seed: with one or
two shooters that can each reach one enemy, all three rules agree. Allocation only matters
when there is a real choice to make, which is why `fire_allocation.toml` had to be built
for this experiment to have anything to measure at all.

### 10.3 Belief-driven sensor tasking *(landed)*

The sim gains a per-side `SpatialBelief` on a coarse grid (`[sim] belief_cells`, default
48), updated each epoch from what the side's sensors *failed* to see and then diffused by
`predict`. This is the point at which the §8.2 POMDP layer stops being a display and
starts driving the simulation.

**The objective is information gain, computed exactly.** For a candidate facing, the
observation is binary per cell: either a sensor detects something at cell `c`, with
probability `b(c)·p(c)`, collapsing the belief to a point mass of zero entropy; or it sees
nothing and the belief becomes `b'(c) ∝ b(c)(1 − p(c))`. So

```
E[H after] = (1 − Σ_c b(c)p(c)) · H(b')
gain(facing) = H(b) − E[H after]
```

and each steerable sensor takes the facing maximising `gain`. Sensors with no
`for_width_deg` see all round and have nothing to choose.

**Why it is affordable.** The expensive part of a detection rate is the line-of-sight
walk, and **LOS does not depend on facing** — only the field-of-regard gate does. So the
per-cell rate is computed once per sensor with the arc removed, cached against the pose it
was built for, and each of the twelve candidate facings is then a cheap arc mask over that
raster. Without this, one epoch would cost a viewshed per facing per sensor.

Carried sensors are excluded: a drone-mounted sensor moves every tick, so it would rebuild
its raster every epoch and never get a cache hit — and it has nothing to steer anyway,
since it faces where its airframe points.

**Off by default (`[sim] sensor_tasking`).** A `facing_deg` written in a scenario is a
statement of intent, and silently overriding it would change what every existing scenario
means. It would also dissolve the §6.3 interdiction game, whose Blue strategies *are*
committed postures — a sensor that re-points itself is no longer playing a strategy. V39
caught exactly this when the default was briefly `true`.

*Measured*, on `scenarios/sensor_search.toml` (three 70°-arc observers, five Red units,
none of them in the sectors the observers start on): a fixed stare finds **2 of 5**; the
belief-driven sweep finds **5 of 5**. Nothing about the sweep is scripted — each sensor
drains its own belief out of ground it has cleared, so the best-information facing moves
on by itself.

### 10.4 Validation gates (V54–V58)

| # | Property | Reference |
|---|----------|-----------|
| V54 | removal preserves history | removal tombstones rather than shifting: every index already in an event log still resolves to the same asset |
| V55 | track lifecycle & EW | a track lapses `track_hold_s` after its last observation and is cleared; continuous observation refreshes it indefinitely; jamming drives `λ_eff` below the maintenance threshold and so *breaks* a track, which permanent detection made impossible |
| V56 | allocation optimality | Hungarian matches an exhaustive brute-force optimum for n ≤ 7; its total payoff is never below greedy's; no target draws more shooters than it has slots; an ineligible pairing is never chosen |
| V57 | tasking beats staring | against an enemy hidden outside its initial arc, a belief-tasked sensor detects where a fixed stare never does, with a shorter mean time-to-detect; belief stays a normalised non-negative distribution with finite entropy across many updates (extends V42); tasking draws no randomness |
| V58 | decision-layer identity | with one shooter and one reachable target, every allocation rule and both tasking settings produce the identical detection and fire logs — the decision phases draw zero randomness, so the stream cannot shift |

**Regression risk, as predicted and as found.** Allocation changes what units shoot at, so
V24, V30 (Lanchester), V31 and V39 were flagged as able to move. **V24, V30 and V31 did
not**, exactly as reasoned: they are single-shooter or homogeneous-line scenarios where
allocation degenerates to the old choice.

**V39 did move**, and for an instructive reason — not allocation, but *tasking*. Its Blue
strategies are committed sensor postures, so a sensor that re-points itself is not playing
a strategy at all, and the "unwatched lane" stopped being unwatched. That is what settled
`sensor_tasking` defaulting to off rather than on. The gate was doing its job: it caught a
model change that would otherwise have quietly invalidated the Phase 6 game.

### 10.5 Deferred

**Movement decisions in-loop** — re-pathing with `least_risk_path` against a live risk
raster. Out of scope for this phase: it needs a per-epoch risk raster, and the coarse-grid
machinery built for §10.3 is what makes that affordable later.
