# Design spec

The deep, quantitative spec: model formulations, equations, state machines, and the
validation reference for each subsystem. The README stays operational; this file holds
the maths. A section is written **before** its subsystem is implemented, and every model
states the analytical result or invariant its tests check against.

Sections are filled in roadmap order. §§1–6 and §8 are written and implemented; §7
(the simulation loop) is a placeholder — the loop is documented alongside the code it
lives in.

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

One primitive, two uses (sensor-coverage display; risk rasters for DP later).
**Brute force first (D6):** run `los` from the observer to every in-range cell. Correct
by construction once `los` is validated; it is the reference oracle any later fast
sweep must match. *Performance pass (2026-07-29): the LOS primitive is now
allocation-free (thread-local scratch, incremental `mask_height`, cached endpoint
elevation) and `viewshed` is parallel over cells (`ndarray` `Zip::par_for_each`, rayon)
— deterministic (each cell writes its own slot; scratch is thread-local). A 3 km-range
viewshed dropped ~9.3 s → 1.8 s, results bit-identical. Brute force stands; no
approximate sweep was needed. Remaining micro-opt: replace the breakpoint sort with an
O(n) merge of the two monotonic crossing streams (marginal vs the sampling cost).* Mobility is
exposed to Phase 5 as `move_cost(from_cell, to_cell)` on cell **edges** (slope
direction matters; uphill penalised harder than downhill; constants are placeholder
dials pending the Phase 5 movement TOML), not a baked isotropic raster.

### 1.6 Validation matrix (the contract — each test names its analytical reference)

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

Detections are permanent in this phase (no track loss — that arrives with EW). Each
detection emits an event `{time, sensor_id, unit_id, unit_pos}` into an append-only log
(the POMDP-ready observation channel, PLAN §5-S3), and flips the unit's
detected-by-the-sensor's-side flag.

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

## 7. Simulation loop *(placeholder)*
Hybrid continuous/discrete structure; epoch semantics; determinism contract (introduced
with the first time-dependent phase).

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
