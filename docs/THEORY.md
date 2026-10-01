# Theory

The quantitative specification: every model in this simulation, worked from the general
form of its mathematics down to the particular form the code evaluates, with the dials it
exposes and the gate that holds it honest.

This is the bottom layer of the documentation. [`README.md`](../README.md) says what the
project is; [`docs/GUIDE.md`](GUIDE.md) says how to set it up, author scenarios and run
experiments; [`docs/MODEL.md`](MODEL.md) walks the same models in plain terms with worked
numbers and the code path for each. **Read those first.** This page is where you come when
you want to know *why* an equation has the form it does, what was rejected on the way to it,
and what it is checked against.

It is one file on purpose. The `§N.M` references scattered through the source – roughly five
hundred of them – resolve here, and landing on the right section of one document beats
landing on a table of contents that then sends you somewhere else.

---

## Contents

| § | Section | Gates |
|---|---|---|
| [0](#0-the-strands) | The strands - each body of theory, and the object it contributes | - |
| [1](#1-terrain-and-line-of-sight) | Terrain and line of sight | V1-V13, V53 |
| [2](#2-fires) | Fires - direct and indirect | V19-V24, V70 |
| [3](#3-sensing-and-detection) | Sensing and detection | V14-V18 |
| [4](#4-suppression-and-attrition) | Suppression and attrition | V28-V31 |
| [5](#5-movement-as-dynamic-programming) | Movement as dynamic programming | V25-V27 |
| [6](#6-the-game-theoretic-layer) | The game-theoretic layer | V32-V39 |
| [7](#7-the-simulation-loop) | The simulation loop - order, determinism, the input contract | V67 |
| [8](#8-electronic-warfare-and-partial-observability) | Electronic warfare and partial observability | V40-V43 |
| [9](#9-air-drones-and-counter-air) | Air: drones and counter-air | V44-V52 |
| [10](#10-the-decision-layer) | The decision layer - tracks, allocation, tasking, movement | V54-V58, V61, V72-V77 |
| [11](#11-command-and-control) | Command and control | V59, V62, V63, V68 |
| [12](#12-sead-air-defence-as-a-target) | SEAD: air defence as a target | V60, V64, V65, V69 |
| [13](#13-the-kill-chain-directed-targeting) | The kill chain: directed targeting | V66 |
| [14](#14-the-measurement-machinery) | The measurement machinery - study design and its estimators | V71 |
| [15](#15-how-the-numbering-works) | How the numbering works | - |

**§7 is the one to read if you read one.** It was written deliberately last: it specifies
the order the models run in and the guarantees that order buys, which only settled once
several phases had each added to it.

## Conventions these sections share

- **Slant range everywhere** (§9.1). Flat ground with equal actor heights reduces to plain
  horizontal distance exactly, which is why adopting it needed no re-baseline.
- **Determinism.** Same binary, same (scenario, seed) produces bit-identical output. Every
  phase added since Phase 3 is *appended* to the loop and draws zero randomness when its
  inputs are empty, so each new subsystem reduces to an exact identity when switched off
  (§7.4).
- **Validated before optimised.** No model is made fast until it has a gate against a closed
  form. Where that meant keeping a slow reference – the brute-force viewshed, the fixed-step
  line-of-sight oracle, the greedy allocator – the reference is kept.
- **Dials are data.** Every number is a TOML dial with a default; these sections state the
  functional form, not the value. The dials themselves are catalogued in
  [`docs/REFERENCE.md`](REFERENCE.md).
- **Placeholder parameters throughout.** No weapon, sensor or unit number here is real
  performance data. The models are the product; the numbers are knobs.
- **Notation.** Mathematics is LaTeX; backticks name something in the source tree: a file, a
  function, a TOML dial. So `move_cost` sits in backticks beside the equation's
  $c_{\text{move}}$, and a symbol carries a gloss naming the dial it corresponds to, because
  that is what ties a formula to the schema. Rust doc comments stay in Unicode, since
  rustdoc typesets no LaTeX.
- **Every section states its own limitations.** What a model deliberately does not do is part
  of its specification, not an admission appended to it.

---

## 0. The strands

Seven bodies of theory, each doing a job the others cannot. This section is the argument for
**why each tool is the right one**, stated in its own symbols; the sections that follow work
each one out against the particular problem it is put to.

Seven here, six in the README: information-gain control (§0.7) is separated out from partial
observability because it is a *control* objective rather than an inference one: it decides where
to look, where the belief filter only decides what to think. The README folds the two together,
which is the right summary for a front page and the wrong distinction to lose here.

| Strand | Canonical object | Realised as | Section | Gates |
|---|---|---|---|---|
| Optimal control | $\dot{x} = f(x,u)$, $u \in U(x)$ | turn-rate-limited flight; phase-integrated orbits | §9.2 | V46, V47 |
| Dynamic programming | $J^*(x) = \min_u [c(x,u) + J^*(f(x,u))]$ | least-risk pathing; Dijkstra as label-setting value iteration | §5, §10.5 | V25-V27, V72-V74 |
| Stochastic processes | Poisson rates, Gaussian dispersion, Markov chains | detection $\lambda$; CEP and Carleton damage; the suppression chain; time-to-kill | §§2-4, §9.4 | V14-V24, V28-V31, V48-V49 |
| Game theory | $v = \max_x \min_y x^{\mathsf{T}} A y$ | zero-sum interdiction game by fictitious play | §6 | V32-V39 |
| Partial observability | belief $b_t(s)$, the posterior over enemy position given $z_{1:t}$ | belief filter with negative information; EW on the rate | §8 | V40-V43 |
| Combinatorial optimisation | $\max \sum_{ij} \text{payoff}[i][j]\, x_{ij}$ over an assignment | side-wide weapon-target allocation (Kuhn-Munkres) | §10.2 | V56 |
| Information-gain control | maximise $H(b) - \mathbb{E}[H(b')]$ over the available looks | belief-driven sensor tasking | §10.3 | V57 |

The simulation loop is **hybrid continuous/discrete**. Between decision epochs the state
integrates continuously; at each epoch the discrete decisions are set: what to shoot, where
to move, where to look. That split is not a convenience, it is the structure: continuous
dynamics are an optimal-control problem, the epoch-to-epoch choices are a dynamic program,
and the two only compose cleanly if they are kept apart. §7.1 makes it concrete.

### 0.1 Optimal control - the continuous dynamics

State evolves under a control that may be chosen but not violated:

$$
\dot{x} = f(x, u), \qquad u \in U(x)
$$

For an airframe the state is $x = (p, \psi, v)$ – position, heading, speed – and the binding
constraint is a **turn rate**, not a position:

$$
|\dot{\psi}| \le \omega_{\max} \quad\Longrightarrow\quad r_{\min} = \frac{v}{\omega_{\max}}
$$

A rate limit is what separates a flight path from a polyline. A drone cannot turn ninety
degrees on the spot; it flies an arc of radius $r_{\min}$ and so arrives late and displaced.
That error is exactly what an air defence's engagement window is made of, so modelling
heading as a state with a bounded derivative is load-bearing rather than decorative.

Orbits integrate the **phase** rather than steering toward the circle:

$$
\theta(t + \Delta t) = \theta(t) \pm \frac{v}{R} \Delta t,
\qquad p = c + R\,(\cos\theta, \sin\theta)
$$

Steering would accumulate radius drift over a long loiter; integrating phase holds
$R = \lVert p - c \rVert$ exactly and gives the lap time $T = 2\pi R / v$ in closed form to
test against. Worked out in §9.2; lives in `airframes.rs`.

### 0.2 Dynamic programming - movement as a least-risk value function

A mover choosing a route is solving Bellman's equation over the terrain grid:

$$
J^*(x) = \min_{u \in U(x)} \Big[ c(x, u) + J^*\big(f(x, u)\big) \Big]
$$

with $x$ a cell, $u$ one of its eight neighbours, and the edge cost

$$
c(x, u) = c_{\text{move}}(x, u) + w \cdot \text{risk}(u)
$$

Because every cost is non-negative there are no negative cycles, so Dijkstra returns $J^*$
exactly: a label-setting value iteration, not an approximation of one. Full value iteration
is held in reserve for when risk becomes time-varying and the one-sweep ordering no longer
holds.

The interesting object is $w$. It is an **exchange rate**: the metres of mobility cost a
commander will spend to avoid one unit of exposure. Sweeping $w$ upward from zero traces the
Pareto frontier between arriving quickly and arriving alive, which is a far more useful
answer than any single "optimal" route. Worked out in §5, and put inside the loop in §10.5;
lives in `movement.rs`.

### 0.3 Stochastic processes - detection, fires, suppression, attrition

Four processes, each chosen so that it has a closed form to test against.

**Detection is a rate**, never a per-tick probability, because memorylessness gives the exact
identity $\prod_k e^{-\lambda \Delta t_k} = e^{-\lambda \sum_k \Delta t_k}$ for any
subdivision of an interval. Halving $dt$ therefore cannot change the detection statistics. A
per-tick probability would silently make the physics a function of the integrator: a class
of bug that is very hard to see and very easy to publish (§3.2, V17).

**Fires are two Gaussians with two closed forms.** Direct fire hits a rectangular silhouette
with probability a product of error functions; indirect fire convolves a Gaussian burst
distribution with a Gaussian damage kernel, which is again a Gaussian and therefore exactly
integrable (§2.2, §2.3).

**Suppression is a birth-death chain**, which hands over its stationary distribution and its
mean first-passage time for free (§4.3).

**Attrition is checked against Lanchester's square law**, the strongest single gate in the
suite, because the law is an *emergent* property of the whole loop rather than of any one
function (§4.1, V30).

**Air defence uses two distributions for one job** – exponential for a gun, geometric for a
missile – because they differ in shape and not merely in mean, so guns and missiles fail
differently against a saturating raid (§9.4).

Lives in `sensing.rs`, `weapon_effects.rs`, `suppression.rs`, `air_defence.rs`.

### 0.4 Game theory - sensing against counter-sensing

Where to put a sensor has no best answer, only a best answer *against a thinking opponent*.
Blue mixes over positions $x \in \Delta_m$, Red over routes $y \in \Delta_n$, and von
Neumann's minimax theorem says the game has a value:

$$
v = \max_{x} \min_{y} x^{\mathsf{T}} A y = \min_{y} \max_{x} x^{\mathsf{T}} A y
$$

Solved by **fictitious play**, which converges for zero-sum games (Robinson, 1951) with no
linear-programming dependency, and certifies its own convergence through a shrinking value
bracket. The payoff matrix is built by the simulation itself, so the equilibrium is a
statement about *this* terrain and *these* sensors rather than about an abstract game.
Worked out in §6; lives in `game_theory.rs`.

### 0.5 Partial observability - belief, and the value of not seeing

Once electronic warfare degrades sensing, the tracked quantity is no longer enemy position
but a **belief** over it, $b_t(s) = P(\text{enemy at } s \mid z_{1:t})$, maintained by the
two standard steps of update and predict.

The load-bearing observation is the **negative** one. Not seeing something is evidence:

$$
P(\text{no detection} \mid \text{enemy at } s) = \exp\big(-\lambda(s)\, \Delta t\big)
$$

A cell a sensor covers well has a low likelihood of producing no detection, so belief drains
out of it; dead ground and jammed cells sit near one and keep their mass. Stare at open
ground long enough and belief mass migrates, unprompted, into the folds and the woodline -
which is what a competent staff officer does with the same information, and it falls straight
out of Bayes rather than being scripted. Worked out in §8; lives in `ew.rs` and `pomdp.rs`.

### 0.6 Combinatorial optimisation - who shoots what

Which shooter engages which target is an assignment problem, and solving it side-wide rather
than shooter-by-shooter is the difference between three tanks all firing at the nearest enemy
and three tanks covering three enemies. Expressing diminishing returns as extra *columns* –
one slot per additional shooter, geometrically discounted – is what keeps this a plain linear
assignment, solvable optimally by Kuhn-Munkres, instead of a submodular problem needing a
bespoke solver. Worked out in §10.2; lives in `allocation.rs`.

A greedy allocator ships alongside, and earns its place by settling what the optimal solver
is actually worth. It turns out to be worth slightly *less* than greedy, for a reason that is
about the objective rather than the solver: §10.6 traces it to the bottom.

### 0.7 Information-gain control - where to look

Entropy is the control objective for the sensor-tasking layer: point each steerable sensor at
the facing maximising the *expected* reduction in $H(b) = -\sum_s b(s) \log b(s)$. That
closes the loop **sensing → belief → decision → action**, and produces an unscripted search
pattern: each sensor drains its own belief out of ground it has cleared, so the
best-information facing moves on by itself. Worked out in §10.3; lives in `sim/tasking.rs`.

### 0.8 Two properties that cut across all seven

- **Determinism.** Same binary, same (scenario, seed) produces bit-identical output.
  Cross-platform bit-equality is not chased; float tests use explicit tolerances. §7.3 states
  the four structural rules that make it hold.
- **Validated before optimised.** No model is made fast until it has a gate against a closed
  form, and optimisations are pinned by bit-identity rather than by "still fast" (§7.4).

---
## 1. Terrain and line of sight

Everything downstream – fires, sensing, movement – reads terrain and calls line of sight, so
the conventions and the query contract here are the highest-blast-radius decisions in the
project. Lives in `terrain.rs` and `los.rs`.

### 1.1 Coordinate and unit conventions

Fixed permanently, because changing one later would silently move every result.

- **World frame:** metres, `f32`. **X = east, Y = north, Z = up.** Right-handed, and it
  agrees with Bevy 2D's Y-up, so the renderer never flips an axis. `f32` resolves about a
  millimetre at 10 km, which is ample at tactical extents.
- **Grid:** an `ndarray::Array2<T>` indexed `[iy, ix]`: row `iy` is northing, column `ix` is
  easting. **Origin at the south-west corner.** A larger index is always further east or
  north, so there is no inversion between grid, world and screen.
- **Registration:** the stored value for cell $(i_x, i_y)$ is the quantity at the **cell
  centre**, at world position $\big((i_x + \tfrac12)s,\ (i_y + \tfrac12)s\big)$ for cell size
  $s$ metres. Continuous values between centres come from bilinear interpolation.
- **One transform.** `GridTransform` is the single place world-to-cell conversion happens
  (`cell_center`, `world_to_cell`, `world_to_frac`). Any GIS raster with a north-west origin
  is flipped **once** at load, never inside a model.
- **Cell size and extent** are scenario parameters. The default is $s = 10$ m over
  $1000 \times 1000$ cells: a 10 by 10 km battlegroup frontage.
- **Determinism.** Terrain and line of sight are pure deterministic functions of their
  inputs. The only randomness is in seeded generation, drawn from the caller's `SimRng`.

### 1.2 The height model - three quantities, never conflated

- **Ground elevation** $z(x,y)$ – bare earth, a DTM. Stored raster, bilinear between centres.
- **Feature height** $f(x,y) \ge 0$ – canopy or building height *above ground*, effectively a
  DSM minus the DTM. Per terrain type, from `terrain_types.toml`. The **blocking surface** is
  $z + f$.
- **Actor height** $h \ge 0$: eye, mast or turret height above the ground the actor stands
  on. An endpoint's absolute height is $z(\text{endpoint}) + h$.

The consequence is that a unit *in* woods sits at $z + h$, under the canopy at $z + f$, not
on top of it. Endpoints are excluded from their own cell's blocking.

This is the definition §9.1 later reuses for airframes: an aircraft's altitude *is* an actor
height, which is why adding air required no change to the line-of-sight query at all.

### 1.3 The terrain grid and its derived layers

`TerrainGrid` owns every layer, so shape invariants live in one place:

- `elevation_m: Array2<f32>` – bare earth $z$.
- `terrain_type: Array2<TerrainType>` – `Open | Trees | Urban`, extensible, `u8` repr.
- **Derived and precomputed at construction** from the per-type dials, all cheap and pure:
  `feature_height_m` ($f$), `cover` $\in [0,1]$, `concealment` $\in [0,1]$, and
  `mobility_cost` $\ge 1$, where `f32::INFINITY` means impassable.

The shipped per-type dials, all placeholders:

| Type | `feature_height_m` | extinction $\kappa$ (per m) | `cover` | `concealment` | `mobility_cost` |
|---|---|---|---|---|---|
| Open | 0 | 0 | 0.0 | 0.0 | 1.0 |
| Trees | 12 | 0.08 | 0.3 | 0.6 | 1.8 |
| Urban | 8 | 0 (hard-blocks) | 0.7 | 0.5 | 1.5 |

**Cover** reduces delivered damage; **concealment** reduces the detection rate; **mobility
cost** multiplies movement cost. They are separate dials because they are separate physical
facts: a wood conceals well and protects poorly, a built-up area does the reverse.

**Terrain sources** sit behind a small seam so later ones are additive: analytic fixtures (flat
from TOML; wall, ridge and hill built in tests, where they double as the line-of-sight validation
terrains), and seeded procedural terrain, all drawn from one RNG stream in fixed order: relief,
then woods, then urban. Relief is a Gaussian-hill sum; woodland is a second hill-sum field
thresholded at the quantile giving the requested fraction; urban is seeded rectangular blocks of
roughly 200-500 m that override woodland. Real DEM (Digital Elevation Model) import is
deliberately out of scope, but the seam keeps it a clean later addition.

**Composable recipes.** `Flat` and `Hills` answer "give me a map"; they do not answer "give me
*this* map". `TerrainSource::Layers` takes a **recipe** – a base surface plus ordered feature
layers (`Ridge`, `Woodland`, `Urban`) – so a map can be described: rolling relief, a ridge
through the middle, light urban. Each layer is a small deterministic operation over
(elevation, terrain type) drawing from the one seeded stream, and **the written order is part
of the contract**: layers apply in sequence, so urban over woodland leaves urban and the
reverse does not. `TerrainSource::Preset` names common recipes – `rolling_hills`,
`wooded_hills`, `light_urban`, `dense_urban`, `mountain_pass`, `flat_plain` – expanding to
the same structure. Sugar, not a second mechanism, so a preset can always be copied out and
adjusted.

`Flat` and `Hills` are deliberately **left as their own arms** rather than re-expressed as
recipes: they consume the RNG in an order that existing scenarios and gates depend on, and
rewriting them would silently change every seeded map.

**Mobility** is exposed as `move_cost(from_cell, to_cell)` on cell **edges**, not as a baked
isotropic raster, because slope direction matters. The slope factor is

$$
s(g) = 1 + 4\max(0, g) + 1.5\max(0, -g),
\qquad g = \frac{\Delta z}{\text{horizontal distance}}
$$

so **flat ground is the cheapest case and both gradients cost more**: ascent about 2.7 times
harder than descent, not descent for free. That is the intended reading: a steep descent is
slow for a tracked vehicle, not an advantage. "Penalises uphill harder than downhill" is easy
to misread as "downhill is cheaper than flat", so it is worth stating that it is not. The two
constants are still `const` in `terrain.rs` rather than dials in a movement stat block: the
one piece of data-drivenness §5 still owes.

### 1.4 Line of sight

The load-bearing primitive. Query: `los(a, h_a, b, h_b) -> LosResult`, for world positions
$a, b$ with actor heights $h_a, h_b$, giving endpoint absolute heights $E_a = z(a) + h_a$ and
$E_b = z(b) + h_b$.

**The sightline is a height profile.** Parameterise the path by horizontal distance
$s \in [0, S]$ with $S = \lVert b - a \rVert$. The sightline height is a closure
$\text{hgt}(s)$; for line of sight it is linear,

$$
\text{hgt}(s) = E_a + (E_b - E_a)\frac{s}{S}
$$

**The same traversal takes a parabolic $\text{hgt}(s)$** for ballistic crest clearance. That
closure is the reason line of sight need not be rewritten for indirect fire: it is why the
signature takes a profile rather than two heights.

**Algorithm.** Amanatides-Woo / DDA grid traversal from $a$ to $b$, visiting exactly the
crossed cells, sampling ground and blocking surfaces by bilinear interpolation at the
traversal points. Symmetric in $a \leftrightarrow b$ by construction, which is a tested
invariant rather than an assumption. A fixed-step sampler with step at most half a cell is
kept as a **test-only oracle** to cross-check against.

**Blocking semantics:**

- **Ground mask (hard):** blocked where $\text{hgt}(s) < z(s)$.
- **Urban (hard):** blocked where the cell is Urban and $\text{hgt}(s) < z(s) + f_{\text{urban}}$.
- **Trees (soft):** accumulate canopy path length $L$, the sum of segment lengths inside
  Trees cells where the sightline passes below $z(s) + f_{\text{trees}}$, giving
  **transmittance** $\tau = e^{-\kappa L}$. $\tau = 0$ if any hard block occurs.

**The return type is rich**, because the traversal computes it all anyway and sensing needs
more than a boolean:

```
LosResult {
    clear: bool,             // no hard (ground or urban) block
    transmittance: f32,      // tau in [0,1]; exp(-kappa L); 0 if hard-blocked
    mask_height: f32,        // extra height at b needed to clear the worst hard mask
                             //   (negative => clearance margin) - defilade reasoning
    blocked_at: Option<f32>, // path distance s of the first hard block
    canopy_length: f32,      // metres of sightline under canopy
}
```

`mask_height` has a closed form. A hard mask of top $T$ at distance $s$ is cleared if and only
if $E_a + (E_b + \Delta - E_a)\tfrac{s}{S} \ge T$, so the required increase in target height
is

$$
\Delta(s) = (T - E_a)\frac{S}{s} - (E_b - E_a),
\qquad \text{mask\_height} = \max_s \Delta(s)
$$

over hard masks, reporting the clearance margin when there are none. This is what makes
defilade reasoning possible: "how much taller would a mast have to be" is answered by the
same query that answers "can it see".

### 1.5 Viewshed

One primitive, two uses: sensor-coverage display, and the risk rasters the movement layer
consumes.

**Brute force**, running `los` from the observer to every in-range cell. Correct by
construction once `los` is validated, and it stays as the reference oracle any faster sweep
must match. It has not needed replacing: two optimisation passes made brute force fast
enough, both bit-identical in output:

| Pass | Change | Measured |
|---|---|---|
| First | LOS made allocation-free (thread-local scratch, incremental `mask_height`, cached endpoint elevation); `viewshed` parallel over cells via `ndarray::Zip` and rayon | 3 km viewshed 9.3 → 1.8 s |
| Second | Breakpoint **sort** removed: each axis's gridline crossings are already ascending in path distance, so the two streams merge in $O(n)$ rather than sorting | LOS 96.8 → 86.7 µs per query; 12 km viewshed 11.5 → 10.4 s |

Parallelism is deterministic: each cell writes its own slot and the scratch is thread-local,
so nothing depends on scheduling. The second pass was worth at least 10% on long rays, since
a multi-kilometre ray carries around two thousand breakpoints; short rays gain nothing. The
figure was measured rather than predicted, which was the point of taking the change.

The two columns above are the before-and-after of each pass, not the current cost.
[`docs/MODEL.md`](MODEL.md#reading-the-benchmark) carries the latest measurements, with the
caveats they need.

### 1.6 Deliberate limitations

- **Terrain is synthetic.** Real DEM ingestion is out of scope by decision, not by
  difficulty. The `TerrainSource` seam is where it would land.
- **`Flat` and `Hills` cannot be expressed as recipes** without changing their RNG
  consumption order, so two mechanisms coexist for describing a map.
- **The slope constants are not dials.** They are `const` in `terrain.rs`, which is the one
  place the data-driven rule is currently broken.

### 1.7 Validation gates (V1-V13, V53)

| # | Property | Checked against |
|---|---|---|
| V1 | World-to-cell round-trip | `world_to_cell(cell_center(c)) == c` for all cells |
| V2 | Bilinear exactness | Sampling an affine field $z = ax + by + c$ returns it **exactly**; bilinear reproduces planes |
| V3 | Derived layers well-formed | cover, concealment $\in [0,1]$; mobility $\ge 1$; no NaN; deterministic from (type, dials) |
| V4 | Generation determinism | Same seed gives a bit-identical raster; a different seed differs |
| V5 | Flat-plane visibility | Any two actors with $h > 0$ on flat open ground: clear, $\tau = 1$ |
| V6 | Single-wall shadow | The hidden zone beyond a wall and `mask_height` match the similar-triangles closed form |
| V7 | LOS symmetry | `los(a,b)` equals `los(b,a)` in `clear` and $\tau$, on random terrain |
| V8 | LOS monotonicity | Raising either endpoint never loses visibility; `mask_height` falls in target height; $\tau$ falls with canopy length |
| V9 | Rigid-motion invariance | Invariant under whole-scenario translation and 90° rotation - catches axis-swap bugs |
| V10 | Canopy extinction law | A uniform Trees strip of width $w$ crossed square-on gives $\tau = e^{-\kappa w}$ exactly |
| V11 | DDA against an oracle | The DDA result agrees with the independent fixed-step sampler within a step-driven tolerance |
| V12 | Flat viewshed is a disc | On a flat plane the viewshed is exactly the in-range cell set |
| V13 | Ridge shadow | For a single infinite ridge, the per-column shadow matches the V6 closed form |
| V53 | Terrain recipes and presets | Recipe plus seed reproduces bit-identically; each layer meets its own invariant (woodland paints its fraction, a ridge lifts its crest by `crest_m`); layer order is significant; presets differ as their names claim |

V53 is numbered out of sequence because composable recipes were added after the air phase. It
is a terrain gate and belongs here, not beside the air gates it was first written next to.

---
## 2. Fires

Direct fire is line-of-sight-gated hit probability; indirect fire is ballistic dispersion plus
area effect plus terrain interaction. Weapons are data-driven stat blocks in
`scenarios/weapons.toml`, and a unit type may carry a `weapon` id. Every model here has a
closed-form gate: this is where "the maths is the product" is most literally true. Lives in
`weapon_effects.rs`.

Each modelling choice below records the alternatives considered, because in most cases the
rejected option was the more obvious one.

### 2.1 Weapon stat blocks

Common to both classes: `class` (`direct` or `indirect`), `rof_rounds_per_min`,
`max_range_m`, and `min_range_m` for indirect weapons.

**Direct:** `dispersion_mrad`, the one-sigma angular aiming error in milliradians, so linear
dispersion at range $r$ is $\sigma(r) = \text{dispersion\_mrad} \cdot r / 1000$ metres;
`p_kill_given_hit`, lethality once a round strikes the silhouette; `moving_target_penalty`, a
factor inflating $\sigma$ against a moving target.

**Indirect:** `cep_m`, circular error probable, taken as constant with range: a dial, with
range-dependent CEP (Circular Error Probable) a documented later refinement; `lethal_radius_m`,
the Carleton damage scale $R_L$.

### 2.2 The direct-fire hit model

Requires `clear` line of sight and $r \le \text{max\_range}$. The round's impact scatters as
an isotropic two-dimensional Gaussian about the aim point – the target centre – with standard
deviation $\sigma(r)$. The target presents a rectangle of width $W$ in deflection, from the
unit's `silhouette_width_m`, and height $H$ in elevation, from its `height_m`.

Deflection and elevation errors are independent, so the hit probability factorises into two
one-dimensional Gaussian integrals:

$$
P_{\text{hit}}(r) = \mathrm{erf}\!\left(\frac{W}{2\sigma(r)\sqrt{2}}\right)
\cdot \mathrm{erf}\!\left(\frac{H}{2\sigma(r)\sqrt{2}}\right)
$$

$$
P_{\text{kill}} = P_{\text{hit}} \cdot p_{\text{kill|hit}}
\cdot \big(1 - \text{cover}(\text{cell}(\text{target}))\big)
$$

`erf` is evaluated by the Abramowitz-Stegun 7.1.26 rational approximation, with maximum error
about $1.5 \times 10^{-7}$ and no new dependency.

*Alternative considered:* a single circular-target form,
$P_{\text{hit}} = 1 - \exp(-R^2/2\sigma^2)$. Rejected, because the rectangular silhouette
gives distinct deflection and elevation behaviour – a tall thin target and a low wide one are
genuinely different problems – and it yields a cleaner Monte Carlo gate.

### 2.3 Indirect-fire dispersion and area effect

The impact point is $b = \text{aim} + N(0, \sigma^2 I)$ with

$$
\sigma = \frac{\text{cep\_m}}{\sqrt{2 \ln 2}} \approx \frac{\text{cep\_m}}{1.1774}
$$

which is the circular-Gaussian CEP identity: the median radial miss of a circular bivariate
normal is $\sigma\sqrt{2 \ln 2}$.

Damage to a target at burst-to-target distance $\rho$ is the **Carleton function**

$$
D(\rho) = \exp\!\left(-\frac{\rho^2}{2 R_L^2}\right)
$$

the standard operational-research incapacitation kernel. For a single round aimed with offset
$d$ from a point target, the expected damage marginalising over the Gaussian burst has a
**closed form**, because a Gaussian convolved with a Gaussian is a Gaussian:

$$
\mathbb{E}[D(d)] = \frac{R_L^2}{\sigma^2 + R_L^2}
\exp\!\left(-\frac{d^2}{2 (\sigma^2 + R_L^2)}\right)
$$

Delivered damage is then multiplied by $\big(1 - \text{cover}(\text{cell})\big)$, reusing the
terrain cover layer so urban and woodland shielding needs no new dial.

**That closed form is the reason for this kernel.** A cookie-cutter lethality disc is simpler
and cheaper, but it has no analytical expectation, so the Monte Carlo sampler would have
nothing to be checked against, and an unvalidated sampler is precisely what this project
exists not to build.

*Alternative considered:* elliptical range and deflection errors (PER/PED), which is more
faithful but needs the gun-target azimuth to orient the ellipse. Deferred; circular CEP
first.

**Crest clearance** reuses the §1.4 traversal with a **parabolic** height profile: the reason
`line_of_sight` takes a profile closure at all. It is wired as a later refinement and is not
active; see the limitation below.

### 2.4 Fires in the simulation loop

A `FireMission { shooter_unit, target }` executes on tick boundaries at the weapon's rate of
fire. Each round draws from the seeded RNG in one fixed-order draw block. Direct rounds sample
hit and kill; indirect rounds sample a burst point and apply $D \cdot (1 - \text{cover})$.
Units carry `strength` $\in [0,1]$ reduced by delivered damage, with `strength <= 0` meaning
killed and removed from sensing and from the live target list. This is the attrition state
validated against Lanchester in §4.

Who shoots what is not decided here: §10.2 allocates fire side-wide, and §13 says what a side
has been ordered to shoot first.

### 2.5 Deliberate limitations

- **No crest clearance.** An indirect round is eligible on range and a track; nothing checks
  that its trajectory would clear the ground between gun and target. A howitzer sited
  immediately behind a high mask therefore fires over it regardless of how high it is, where a
  real minimum-ordinate calculation would refuse the mission or demand a different charge.
  This is deliberate: the arc is not simulated, only its endpoints, so there is no ordinate to
  compare against. It matters only for a gun sited very close to very steep ground.
- **Eligibility is a track, not a sightline**, which is correct, and worth stating because it
  is the mirror of direct fire and easy to assume the other way round. A howitzer engages what
  its side can *see*, not what *it* can see. V70 pins both directions; §13.4 covers what that
  means for a fire plan.
- **CEP does not vary with range.** A real gun's dispersion grows with range and charge.

### 2.6 Validation gates (V19-V24, V70)

| # | Property | Checked against |
|---|---|---|
| V19 | Direct-fire hit probability | Monte Carlo fraction of impacts inside the $W \times H$ rectangle, within a binomial CI of the erf product |
| V20 | Hit-probability monotonicity | Falls with range and cover, rises with target size; exactly 0 when blocked or beyond maximum range |
| V21 | Indirect CEP | Empirical median miss distance of sampled bursts equals `cep_m` within CI (the Rayleigh median) |
| V22 | Area-damage closed form | Monte Carlo mean Carleton damage over sampled bursts equals $\mathbb{E}[D(d)]$ within CI, swept over $d$ |
| V23 | Damage monotonicity | $\mathbb{E}[D]$ falls with offset $d$ and with cover, rises with `lethal_radius_m` |
| V24 | Fires determinism | Same (scenario, seed, mission) gives identical round outcomes and final strengths |
| V70 | Indirect eligibility is a track, not a sightline | A howitzer engages a target masked from it by a ridge, because a shell arcs and what it needs is a track somebody on its side holds; a direct-fire gun in the identical position holds its fire. The converse binds too: jamming the observer drives the glimpse rate below `track_maintain_p`, the track ages out, and the indirect shooter's lock is released with it - while the target is still alive, so the release is the lapse and not the kill |

---

## 3. Sensing and detection

The core interactive loop: place sensing assets and try to detect the enemy before being
detected. Detection is **mutual and asymmetric** – both sides' sensors run the same machinery
against the other side's units – so positioning to see without being seen is a real decision
rather than a scoring bonus. Lives in `sensing.rs`.

### 3.1 Stat blocks

**Sensor type** (`scenarios/sensors.toml`): `modality`, an enum: `optical` now, with `acoustic`
and `eo_ir` later, each bringing its own propagation term, which is why it is a tag rather than a
convention; `mount_height_m`, so a mast or a hovering UAS (Uncrewed Aerial System) is just this
dial; `max_range_m`, a hard cutoff; `lambda0_per_s`, the peak detection rate against signature 1
at $\tau = 1$, zero concealment and point-blank range; `range_half_m` and `range_exponent`, the
falloff curve; and an optional field of regard `for_width_deg`, with the sensor *instance*
carrying the facing. Omitting the arc means all-round.

**Unit type** (`scenarios/units.toml`): `height_m` as the target height for line of sight, and
`signature` as a **per-modality table** – `signature.optical = 0.6` – so acoustic and EO/IR
add keys rather than schema changes.

### 3.2 The glimpse-rate model

Detection of unit $u$ by sensor $s$ is a Poisson process with rate

$$
\lambda(s, u) = \lambda_0 \cdot f(r) \cdot \sigma_m(u) \cdot \tau(s, u) \cdot \big(1 - c(u)\big)
$$

$$
f(r) = \frac{1}{1 + \left(r / r_{1/2}\right)^{n}}
$$

where $\lambda_0$ is `lambda0_per_s`, $\sigma_m$ is the target's signature in the sensor's
modality, $c$ is terrain concealment at the target's cell, $r_{1/2}$ is `range_half_m`, $n$ is
`range_exponent`, and $\tau$ is the canopy transmittance from the §1.4 query. $r$ is **slant**
range (§9.1).

The rate is gated to exactly zero when line of sight is not `clear`, when
$r > \text{max\_range}$, or when the target is outside the field of regard. $\tau$ and `clear`
both come from the one line-of-sight call, with the sensor's `mount_height_m` and the unit's
`height_m` as endpoint heights: this is what the rich `LosResult` was built for.

Then

$$
P(\text{detected by } t) = 1 - e^{-\lambda t},
\qquad \text{per tick: } p = 1 - e^{-\lambda \Delta t}
$$

**Modelling a rate rather than a per-tick probability is the load-bearing choice**, and it is
worth being explicit about why. Memorylessness gives the exact identity

$$
\prod_k e^{-\lambda \Delta t_k} = e^{-\lambda \sum_k \Delta t_k}
$$

for any subdivision of the interval, so halving $dt$ cannot change the detection statistics. A
per-tick probability would silently make the physics a function of the integrator, which is a
very hard bug to see and a very easy one to ship. V17 checks the identity to float tolerance.

Each detection emits an event `{time, sensor_id, unit_id, unit_pos}` into an append-only log –
the observation channel the §8.2 belief filter consumes – and marks the unit detected by the
sensor's side.

Detections were permanent in the first version of this model, with track loss deferred to EW
(Electronic Warfare). **§10.1 replaced that**: a track is held only while it is re-observed and
ages out `track_hold_s` after its last observation. Acquisition is still the stochastic glimpse
described here; only maintenance was added.

### 3.3 The two clocks, first appearance

`Sim::new(&scenario, &libraries, seed)` resolves stat blocks and builds `SimState`; `Sim` owns
the `SimRng`. `step_one()` advances one tick of `dt_s`, and every `epoch_s` of simulated time
a decision hook runs. The scenario `[sim]` block sets both, defaulting to `dt_s = 1.0` and
`epoch_s = 10.0`. §7.1 specifies the arrangement in full.

Determinism here: sensors and units iterate in fixed index order, there is one RNG draw per
live (sensor, undetected-unit) pair per tick, and state carries no hash-ordered containers.

### 3.4 Deliberate limitations

- **One modality is implemented.** The `Modality` tag is the seam for acoustic and EO/IR, and
  acoustic detection of drones is the obvious next one.
- **Signature is a scalar per modality.** There is no aspect dependence, no thermal crossover,
  no speed-dependent signature.
- **Concealment is a property of the cell**, so an airborne target contributes zero (§9.1).

### 3.5 Validation gates (V14-V18)

| # | Property | Checked against |
|---|---|---|
| V14 | Detection-time distribution | For a constant-$\lambda$ pair, the empirical mean over seeded Monte Carlo runs equals $1/\lambda$ within CI |
| V15 | Detection closed form | Monte Carlo frequency of detection by time $t$ lies within a binomial CI of $1 - e^{-\lambda t}$ |
| V16 | Rate structure | $\lambda$ is monotone non-increasing in range and concealment; exactly 0 when blocked, out of range or outside the field of regard; linear in signature and in $\tau$ |
| V17 | **Tick-size invariance** | Compounded per-tick survival $\prod e^{-\lambda \Delta t}$ equals $e^{-\lambda t}$ for $\Delta t \in \{0.25, 0.5, 1, 2\}$, to float tolerance |
| V18 | Sensing determinism | Same (scenario, seed) gives an identical event log; a different seed differs |

V17 is the one that earns its keep. It is what makes the answer independent of the integrator,
and it is the gate that would fail first if detection were ever reformulated as a per-tick
chance.

---
## 4. Suppression and attrition

Units are **N sub-elements** attriting one at a time; suppression is a **discrete
Free/Suppressed/Pinned Markov chain** driven by near-miss volume; the whole thing is validated
against **Lanchester's square law**. Lives in `suppression.rs`, with attrition in
`weapon_effects.rs`.

### 4.1 Units as N elements

A unit type carries `element_count`, defaulting to 1. `UnitState` tracks `elements` remaining
of `initial_elements`, with `strength() = elements / initial` for display and
`alive() = elements > 0`. Attrition removes whole elements:

- **Direct round:** one round that passes
  $p_{\text{hit}} \cdot p_{\text{kill|hit}} \cdot (1 - \text{cover})$ removes **one** element.
- **Indirect round:** a burst delivers Carleton damage $D$ to the unit, and each surviving
  element is independently killed with probability $D \cdot (1 - \text{cover})$ – a binomial
  draw – so area fire attrits a group properly, and expected casualties are
  $\text{elements} \cdot D \cdot (1 - \text{cover})$.

**Fire volume scales with the shooter's live elements.** A unit fires
$\text{round}(\text{rof} \cdot \text{epoch}/60) \times \text{elements}$ rounds per epoch,
because each element shoots. This is precisely what makes the aimed-fire duel obey Lanchester's
square law:

$$
\frac{dA}{dt} = -\beta B, \qquad \frac{dB}{dt} = -\alpha A
\qquad\Longrightarrow\qquad
\alpha\big(A_0^2 - A^2\big) = \beta\big(B_0^2 - B^2\big)
$$

Combat power goes as the *square* of numbers under aimed fire. Reproducing it is the strongest
single check in the suite, because it is an **emergent** property of the whole loop – element
counts, rate of fire, hit probability, removal – and not of any one function. Nothing was
written to produce it.

### 4.2 Fire gating

- **Direct fire:** engage an enemy in clear line of sight and range, *no detection required*.
  You shoot what you can see.
- **Indirect fire:** engage a **detected** enemy in range, with no line of sight required,
  because the round arcs. You bombard where you have been cued.

The asymmetry is the whole content of V70, and §13.4 covers what it means for a fire plan: a
masked top-priority target blocks a direct-fire gun and does not block the howitzer behind it.

### 4.3 Suppression as a birth-death chain

Per-unit state $S \in \{\text{Free}, \text{Suppressed}, \text{Pinned}\}$. Near-misses – rounds
landing within `suppression_radius_m` of the unit that do **not** kill it – push the state up;
time pushes it back down. Per near-miss the state steps up one level with probability
`p_suppress`; each tick it steps down one level at rate `recover_per_s`, a memoryless recovery
timer.

$$
\text{Free} \rightleftharpoons \text{Suppressed} \rightleftharpoons \text{Pinned}
\qquad \pi_k \propto (\beta/\mu)^k, \quad k = 0, 1, 2
$$

The birth-death structure hands over its stationary distribution for free, and the mean time
from Pinned to Free is $2/\mu$: two exponential steps in series. Both are closed forms to gate
against, which is why the chain is discrete rather than a continuous suppression scalar.

Effects, and the reason the state is discrete:

- **Free:** normal.
- **Suppressed:** outgoing fire effectiveness multiplied by `suppressed_fire_factor` $< 1$;
  may still move.
- **Pinned:** cannot fire and cannot move.

Suppression is a **state** rather than a scalar multiplier because it gates behaviour
discontinuously. Pinned units neither fire nor move, which is what lets fires shape manoeuvre
without killing anybody, and that, rather than attrition, is what artillery mostly does.

**What counts as a near-miss differs by weapon class, and the difference is a simplification
worth stating.** Indirect fire samples an impact point and tests it: a round is a near-miss
when $\lVert \text{burst} - \text{target} \rVert < \text{suppression\_radius\_m}$. **Direct
fire does not.** It resolves a single hit/kill roll and treats *every* miss as a near-miss,
whatever the range, so `suppression_radius_m` does not enter the direct-fire path at all.

At the shipped dials this is very nearly exact: 0.4 mrad at 3 km is $\sigma \approx 1.2$ m, so
essentially every direct-fire miss really does land inside a 35 m radius. It stops being exact
for a high-dispersion or very long-range direct weapon, where suppression would scale with
*rounds fired* rather than with rounds landing close.

It is left as it is deliberately. Sampling an impact point for direct fire would change the
number of RNG draws per round, which re-baselines V24, V30 and V31: a real cost for an effect
the current dials cannot resolve.

### 4.4 Deliberate limitations

- **Suppression has three levels, not a continuum.** That is a modelling choice made for the
  closed forms it buys, not an approximation of something finer.
- **Direct fire suppresses on rounds fired, not rounds landing close**: see above.
- **There is no morale, cohesion or command effect on suppression**, and no unit-to-unit
  variation in susceptibility.

### 4.5 Validation gates (V28-V31)

| # | Property | Checked against |
|---|---|---|
| V28 | Chain stationary distribution | Under a constant near-miss rate $\lambda_{nm}$ and recovery $\mu$, long-run occupancy of the three states matches the analytic birth-death stationary distribution within CI |
| V29 | Recovery time | With no incoming fire, mean time Pinned to Free is $2/\text{recover\_per\_s}$ - two exponential steps - within CI |
| V30 | **Lanchester square law** | An aimed direct-fire duel with no terrain and no suppression reproduces $\alpha(A_0^2 - A^2) = \beta(B_0^2 - B^2)$: many-trial mean force curves match the ODE solution within CI |
| V31 | Suppression gates fire | A Pinned unit emits no rounds; a Suppressed unit's expected output equals `suppressed_fire_factor` times a Free unit's |

---

## 5. Movement as dynamic programming

Least-risk pathing over the terrain grid: a mover trades **mobility cost** – time and effort –
against **exposure risk**. The value function is exactly a shortest-path problem, so Dijkstra
over the cell graph *is* the dynamic-programming solution here. Lives in `movement.rs`.

### 5.1 Formulation

An 8-connected grid graph, where each edge costs

$$
c(\text{from} \to \text{to}) = c_{\text{move}}(\text{from}, \text{to}) + w \cdot \text{risk}(\text{to})
$$

`move_cost` is the §1.3 terrain edge cost: mean mobility times slope factor times distance,
infinite when impassable. `risk(cell)` is a caller-supplied exposure raster in $[0,1]$, and
`risk_weight` $w$ tunes caution: the metres of mobility cost the mover will spend to avoid one
unit of risk.

The least-cost path minimises the total $\sum \text{edge cost}$, an **additive** cost so the
problem is a clean shortest path. Solved by Dijkstra with a binary heap, skipping infinite
edges, returning the path and its total cost, or `None` if the goal is unreachable.

*Alternative considered:* maximising multiplicative survival $\prod(1 - p_{\text{death}})$.
Richer, but the logarithm turns it into the same additive problem anyway, and additive keeps
the Dijkstra gate clean.

**Why Dijkstra is the DP (Dynamic Programming) and not an approximation of it.** Every edge cost
is non-negative, so there are no negative cycles and label-setting is exact: the first time a node
is settled, its label is $J^*$. Full value iteration is held in reserve for the case where risk
becomes time-varying, at which point the one-sweep ordering no longer holds.

### 5.2 The risk raster

Risk defaults to **enemy observation coverage**: for each cell, the detection rate a reference
mover would suffer from the best-placed enemy sensor – the maximum over enemy sensors of the
§3.2 rate against a reference unit – normalised to $[0,1]$.

This reuses the sensing model, so "least-risk path" literally means "the route that stays
hardest to see", and the see-without-being-seen idea becomes navigable. The path solver itself
is agnostic: any $[0,1]$ raster works, which is what let §10.5 later add a fire term without
touching the solver.

**Sweeping $w$ is the point.** A single "optimal" route answers a question nobody asked. The
frontier traced by $w$ from zero upward – arriving quickly against arriving alive – is the
answer with operational content.

### 5.3 Deliberate limitations

- **The reference mover is not the unit.** Risk is a property of the ground and the enemy's
  sensors, computed once per side, so an unusually stealthy unit is not routed differently
  from a conspicuous one.
- **Risk is static within an epoch.** §10.5 re-solves each epoch, which is the practical
  answer, but the solver itself assumes a fixed raster.

### 5.4 Validation gates (V25-V27)

| # | Property | Checked against |
|---|---|---|
| V25 | Zero risk gives the shortest path | With `risk_weight = 0` on uniform-mobility flat terrain, path cost equals the closed-form 8-connected distance $(\max - \min) + \sqrt{2}\min$, scaled by cell mobility |
| V26 | Risk avoidance is monotone | Raising `risk_weight` never increases total risk exposure along the optimum; a high-risk barrier is routed around once the weight is high enough |
| V27 | Path optimality | Dijkstra's cost matches an independent Bellman-Ford reference on a small grid; the path is contiguous, in bounds and endpoint-correct |

A note on `Ord` and `Eq` for the frontier: they must agree, including for NaN, because
`BinaryHeap` may rely on the total order. A disagreement there is not a hypothetical: it is
the kind of defect that produces a wrong path on one input in ten thousand and passes every
test that does not happen to hit it.

---

## 6. The game-theoretic layer

The capstone of the pre-decision-layer phases: it exercises terrain, line of sight, sensing,
movement, fires and suppression through one solved zero-sum game. Lives in `game_theory.rs`.

### 6.1 Movement in the simulation

Units carry a route and a speed. `UnitType.speed_m_s` defaults to 0, meaning static;
`UnitState` carries `route: Vec<Vec2>` and `route_idx`. Each tick a live **unpinned** unit
advances $\text{speed} \cdot dt$ metres along its polyline, consuming multiple segments if a
tick's travel spans them; a **Pinned** unit does not move, which is suppression gating
movement. Reaching the last waypoint halts it. Detection and fires use the updated positions,
which is why movement leads the tick (§7.2).

### 6.2 The zero-sum solver: fictitious play

For a payoff matrix $A$ with rows the maximiser (Blue) and columns the minimiser (Red),
fictitious play alternates best responses to the opponent's empirical play:

$$
i^* = \arg\max_i \sum_j A_{ij} N^{\text{col}}_j,
\qquad
j^* = \arg\min_j \sum_i A_{ij} N^{\text{row}}_i
$$

incrementing the count of each chosen strategy. Time-averaged strategies converge to a Nash
equilibrium and the value converges (Robinson, 1951, for zero-sum games), with no
linear-programming dependency.

**Convergence is self-certifying.** The value is bracketed by

$$
v_{\text{low}} = \min_j \big(x^{\mathsf{T}}A\big)_j \le v \le \max_i \big(Ay\big)_i = v_{\text{high}}
$$

- Blue's guarantee below, Red's above, and the gap $v_{\text{high}} - v_{\text{low}}$ shrinks
to zero. As a result, the algorithm reports its own error rather than being run for a fixed number of
iterations and hoped over.

### 6.3 The interdiction payoff

- **Blue strategy:** a position $b$ holding a sensor and a co-located **observed indirect**
  shooter. Detection gates its fire, since indirect fire needs a track.
- **Red strategy:** a route $r$: candidate paths across the map, some from the §5 least-risk
  pather at varying caution, some direct.
- **Payoff $A[b][r]$:** expected Red attrition, as a fraction of Red elements lost, when Red
  traverses $r$ while Blue at $b$ watches and bombards. Estimated by a short headless Monte
  Carlo battle averaged over seeds. Blue maximises, Red minimises, zero-sum in the attrition
  metric.

The matrix is built once, then fictitious play solves it, so the solver stays cheap even though
payoff construction is the expensive part. Kept tractable with strategy sets of six to eight
each and short battles, all in the Bevy-free `experiments` crate.

**The matrix is built by the simulation itself**, which is what makes the equilibrium a
statement about *this* terrain and *these* sensors rather than about an abstract game.

### 6.4 Deliberate limitations

- **The game is static.** Strategies are committed postures chosen once, not policies that
  respond to what the other side does mid-run. A dynamic or stochastic game, with DP value
  functions supplying the payoffs, is the natural next step.
- **Strategy sets are small and hand-chosen.** They must be, because each cell of the matrix
  costs a batch of simulations.
- **Blue's strategies are committed postures, which is why `sensor_tasking` defaults off**
  (§10.3). A sensor that re-points itself is not playing a strategy at all, and V39 catches
  exactly that.

### 6.5 Validation gates (V32-V39)

| # | Property | Checked against |
|---|---|---|
| V32 | Matching pennies | Fictitious play value tends to 0, both strategies to $(\tfrac12, \tfrac12)$ |
| V33 | Rock-paper-scissors | Value tends to 0, both strategies to uniform |
| V34 | Saddle point | A game with a pure equilibrium converges to that value, with deterministic strategies |
| V35 | Strict dominance | A strictly dominated strategy converges to approximately zero weight |
| V36 | Skew-symmetric fairness | $A = -A^{\mathsf{T}}$ implies value 0; the value bracket closes |
| V37 | Route following | A unit on a straight route is at $\text{speed} \cdot t$ after $t$ seconds, to within one tick |
| V38 | Pinned unit halts | A Pinned unit does not advance along its route |
| V39 | Interdiction sanity | A route outside every Blue position's view is safe, so Red weights it and the value falls; a Blue position covering all routes raises the value |

---
## 7. The simulation loop

Every other section specifies a *model*; this one specifies the **order those models run in
and what that order guarantees**, which is the thing most easily broken by accident and
hardest to notice afterwards. It was written last, after several phases had each added to the
loop. Lives in `sim/mod.rs`, with the phases split across `sim/`.

**If you read one section, read this one.**

### 7.1 Two clocks, and why

The loop is hybrid continuous/discrete:

- **The tick** (`dt_s`, default 1 s) integrates what changes continuously: movement, and the
  moment-to-moment hazard of being seen.
- **The decision epoch** (`epoch_s`, default 10 s) is where *choices* are made: what is still
  being tracked, where to look, what to shoot, where to move.

The split is the optimal-control-plus-dynamic-programming structure of §0 made concrete, and
it is load-bearing in both directions. Physically, fire missions are not re-planned sixty
times a minute. Computationally, it is what keeps the expensive decision layer – an assignment
solve and an information-gain search – off the hot path: §10.1 measured track maintenance at
about 2.3 ms per tick against about 0.23 ms at epoch cadence.

A tick may straddle an epoch boundary, or several. `step_one` advances the clock and then
resolves *every* boundary the new time has crossed, so `epoch_s` need not be a multiple of
`dt_s`, and a coarse `dt_s` cannot silently skip a decision.

### 7.2 Phase order

The authoritative list, as `Sim::step_one` runs it.

| # | Phase | Draws RNG? | Spec |
|---|---|---|---|
| 1 | Ground movement along routes | no | §6.1 |
| 2 | Air movement, then carried-sensor sync | no | §9.2, §9.6 |
| 3 | Sensing against enemy ground units | **yes** - one draw per eligible pair | §3.2 |
| 4 | Sensing against enemy air | **yes** - one draw per eligible pair | §9.1 |
| 5 | Suppression recovery | **yes** - one draw per non-Free unit | §4.3 |
| 6 | C2 link refresh | no | §11.1 |
| 7 | Air-defence resolution | **yes** - per engagement due | §9.4 |
| 8 | Strike release | **yes** - burst point, damage rolls | §9.3 |
| 9a | Track maintenance | no | §10.1 |
| 9b | Sensor tasking | no | §10.3 |
| 9c | Movement re-planning | no | §10.5 |
| 9d | Fire allocation, then resolution | allocation no, rounds **yes** | §10.2, §2 |

Phases 1-8 run every tick; phase 9 only on an epoch boundary. Four orderings are
constraints, not preferences.

**Movement leads the tick**, so that sensing and fires act on current positions rather than
last tick's. A target that moved into cover this tick is in cover when it is looked at.

**Tracks are maintained before tasking**, because tasking reasons about what was *not* seen
this epoch.

**Tasking precedes fires**, because indirect fire is gated on tracks. A sensor that loses
contact silences the guns behind it, and that must be visible in the same epoch rather than
one epoch later.

**The C2 (Command and Control) link is refreshed before air defence resolves**, because who is in
the net decides how the batteries allocate. It is a no-op without posts and draws no randomness
either way.

### 7.3 The determinism contract

**Same binary, same (scenario, seed) produces bit-identical output.** Cross-platform
bit-equality is explicitly *not* promised, because floats; float comparisons in gates use
stated tolerances.

Four structural rules make it hold, and each is enforced somewhere rather than trusted:

1. **One seeded stream.** All randomness comes from the `SimRng` the `Sim` owns: `ChaCha8Rng`,
   chosen because its stream is stable across `rand` versions, so an archived seed still
   reproduces after a routine dependency bump. No wall clock, no thread RNG, no global state.
2. **Fixed iteration order.** Assets are visited by index, never by hash order, and state
   carries no `HashMap` whose iteration reaches a result. Placement order is itself part of
   the contract, so two runs agree on every index.
3. **Parallelism writes disjointly.** The rasters – viewshed, coverage, belief, risk – are
   computed with rayon via `ndarray::Zip`, where each cell writes its own slot and the
   line-of-sight scratch is thread-local. No result depends on scheduling.
4. **New phases are appended and draw nothing when idle.** This is the rule that has let the
   loop grow from three phases to eleven without invalidating what came before.

### 7.4 The identity discipline

Rule 4 deserves stating as a design *method*, because it is how every subsystem since Phase 3
has been added without re-baselining anything.

> A new subsystem must reduce to an **exact identity** when it has nothing to do, not an
> approximation, not "close enough". Switched off, the event log is bit-identical to the build
> before it existed.

The stronger claim is deliberate. An approximate identity hides a real change inside sampling
noise, and there is no threshold at which it can be distinguished from a bug.

Each such claim gets its own gate rather than being asserted:

| Subsystem | Identity when | Gate |
|---|---|---|
| Electronic warfare | no jammers, so every factor is exactly 1 | V40 |
| Air and counter-air | no air assets, so phases 2, 4, 6, 7 draw zero RNG | V52 |
| Decision layer | one shooter and one reachable target, so all rules agree | V58 |
| C2 | no post, so coordination changes nothing | V59 |
| Ground fires needing C2 | dial off, so the fire log is bit-identical | V63 |
| Anti-radiation homing | no flag, so the emitter is ignored entirely | V64 |
| Counter-battery | no enemy emplacements, so the target list is unchanged | V65 |
| Doctrine | one tier over everything, which *is* the undirected behaviour | V66 |
| Movement decisions | no objective declared, so no planner is built at all | V72 |
| Allocation horizon | $H = 1$ multiplies threat by exactly 1.0 | V75 |
| Line-of-sight memoisation | a cache hit is the value a miss would have computed | unit tests |

**Where possible the identity is structural rather than dial-gated**, meaning the new code does
not run at all rather than running with its effect set to zero. Movement decisions build no
planner when no unit declares an objective; doctrine defaults to a single tier over everything.
A structural identity cannot be broken by someone changing a default.

The payoff is concrete: adding drones did not move a single ground-scenario result, and sensor
tasking could be added without touching the §6 game: once it was defaulted off, which V39 is
what forced.

**The discipline also constrains optimisation**, not just modelling. Both major performance
passes were verified by hashing a multi-scenario, multi-seed batch before and after and
requiring the digest to match. That is why the indirect damage factors are deliberately *not*
pre-multiplied into one term: float multiplication is not associative, and folding them would
shift a result by an ulp: enough to flip a knife-edge kill roll and silently re-baseline V22
and V24.

### 7.5 What the loop does not do

Stated so the boundaries are visible rather than assumed.

- **No intra-epoch reaction.** Both sides allocate against the board as it stood at the start
  of the epoch, so neither reacts to casualties the other has not taken yet. This is a
  deliberate simultaneity choice.
- **No variable time step.** A finer `dt_s` costs proportionally more and, because detection is
  a rate rather than a per-tick probability (§3.2), buys no accuracy in the detection
  statistics, which is exactly what V17 checks.
- **No terrain-aware communications.** The C2 link is degraded by jamming and by latency
  (§11.1) but not by a ridge between post and battery.

### 7.6 The input contract

The schema's `deny_unknown_fields` refuses a **key** the model does not know, on the grounds
that a misspelt dial takes its default and produces a study of a different question: a failure
invisible because the run *succeeds*. A **value** outside its domain fails in exactly the same
way.

`Scenario::validate` and `Libraries::validate` refuse both, naming the offending dial. Two of
the refusals are of a different order from the rest, and are the reason this section exists:

```
dt_s    = 0   =>  the clock never advances, so run_until never returns
epoch_s = 0   =>  time_s / epoch_s is +inf, and `as u64` SATURATES rather than wrapping,
                  so the epoch loop is handed u64::MAX boundaries to resolve
```

Neither is exotic. `sweep` exists precisely to set any dotted path in a file from the command
line, so `--param sim.epoch_s --from 0 --to 30` is an ordinary-looking sweep whose first arm
hangs with no diagnostic at all.

The rest are ordinary domain checks: probabilities in $[0,1]$; `track_hold_s`, `recover_per_s`,
`suppression_radius_m`, `risk_weight`, `fire_risk_weight` and `repath_margin` non-negative;
`belief_cells` and `allocation_horizon` at least 1: plus the small set of stat-block dials that
reach a **divisor**: a sensor's `range_half_m` in the §3.2 falloff, and an indirect weapon's
`lethal_radius_m` in the §2.3 Carleton kernel. Those two are singled out because a zero there does
not give a small answer, it gives `NaN`, and `NaN` loses every comparison it appears in, so the
subsystem goes silently *inert* rather than visibly wrong, which is the hardest kind of failure to
notice.

**The list is deliberately short.** Most dials being zero is a legitimate statement, and
refusing them would break real fixtures: a drone with `cruise_speed_m_s = 0` is stationary,
which V59 and V62 depend on to keep their geometry from drifting; a battery with
`max_range_m = 0` engages nothing; a direct weapon never touches the Carleton kernel, so its
unused `lethal_radius_m` of zero means nothing at all. A validator that refused those would be
enforcing taste rather than tractability.

`Libraries::validate` runs both at load and again inside `Sim::new`, so a library patched in
memory – which is what `sweep` does – is held to the same contract as one read from disk.

### 7.7 Validation gate (V67)

| # | Property | Checked against |
|---|---|---|
| V67 | A dial the model cannot run on is refused at load, naming it | `dt_s` or `epoch_s` at zero or negative, both of which fail to *terminate*; probabilities outside $[0,1]$; negative durations and radii; `belief_cells = 0`; and any stat-block dial reaching a divisor where a zero yields `NaN`. Legitimate zeros still load, and every shipped scenario and library satisfies the contract |

---

## 8. Electronic warfare and partial observability

EW degrades sensing; with imperfect detection the observer reasons over a **belief** – a
probability distribution – rather than over ground truth. EW is a clean modifier on the sensing
channel: with no jammers it is the identity, so EW-off reduces bit-for-bit to §3. Lives in
`ew.rs` and `pomdp.rs`.

### 8.1 Jamming as a modifier on the rate

A jammer protects its own side's units by degrading the enemy's detection of them. A jammer at
$p$ with `power` $\in [0,1]$ and `radius_m` contributes, inside its radius,

$$
g_j = 1 - \text{power}_j\left(1 - \frac{d}{\text{radius}_j}\right)
$$

so $1 - \text{power}$ at the centre and exactly 1 at the edge. Factors compose
multiplicatively, and the glimpse rate becomes

$$
\lambda_{\text{eff}} = \lambda \cdot \prod_j g_j(\text{target})
$$

over the *target's own side's* jammers.

**EW enters the rate, not the geometry.** Nothing moves; a jammer raises the belief entropy
$H(b)$ without changing where anything is. With no jammers every $g_j$ is exactly 1 and the
product is exactly 1, so EW-off is an exact identity (V40) rather than a near-identity.

Note the sign convention, because §11.1 uses the mirror image of it. `Sim::jamming_at` folds a
side's **own** jammers, because a jammer protecting Red degrades *Blue's sensing of Red*.
`Sim::link_quality_at` folds the **enemy's**, because a jammer degrades *Blue's own
communications*. Same asset, same dials, opposite side of the argument, so one Red jammer both
hides Red units and cuts the Blue net.

### 8.2 The belief filter

An **inference layer over the simulation**, not simulation state. `bayes_update(prior,
likelihood)` is the generic discrete posterior; a `SpatialBelief` holds a per-cell distribution
over enemy position with `update` (Bayes against an observation-likelihood raster), `predict`
(a diffusion motion model that raises entropy), `entropy` and `most_likely_cell`.

The two standard steps:

$$
\text{update:} \quad b_t(s) \propto P(z_t \mid s)\, b_{t-1}(s)
$$

$$
\text{predict:} \quad b^{-}_t(s') = \sum_s T(s' \mid s)\, b_{t-1}(s)
$$

**The key observation model is negative information.** `no_detection_likelihood` gives, per
cell, $P(\text{no detection} \mid \text{enemy there})$, reusing the sensing model *and* EW:

$$
P(\text{no detection} \mid \text{enemy at } s) = \exp\big(-\lambda_{\text{eff}}(s)\, \Delta t\big)
$$

Cells the sensor covers well get a low likelihood – the enemy would have been seen – so belief
drains out of them; dead ground and jammed cells sit near 1 and keep their mass. Multiplying an
uninformative prior by this likelihood over the exposure window *is* "where an undetected enemy
could be", which is what the app's belief heatmap shows: cleared coverage against plausible
hiding ground.

Stare at open ground long enough and belief mass migrates, unprompted, into the folds and the
woodline. That is what a competent staff officer does with the same information, and here it
falls straight out of Bayes rather than being scripted.

### 8.3 Deliberate limitations

- **`predict` is not mass-conserving at the map edge.** It mixes each cell with the mean of its
  existing neighbours, which conserves mass in the interior but leaks at the boundary, where a
  cell has fewer neighbours than the cells feeding it; the renormalisation afterwards absorbs
  the difference. Two consequences, both small and both worth knowing before a rim artefact is
  read as a finding: the diffusion rate is not exactly the fraction of mass that moves, and
  repeated prediction drifts belief slightly toward the map interior with no evidence behind
  it. At the §10.3 grid of 48 by 48 over a few dozen epochs this is far below the resolution
  any decision is taken at.
- **Belief is over enemy *position*, and never folds in a positive detection.** It is
  maintained from what was *not* seen. §10.3's tasking objective is derived assuming a belief
  over an *undetected* enemy, and the distinction is worth a decision rather than an
  assumption.
- **Jamming has no directionality, no frequency and no burn-through.** Power and radius are the
  whole model.

### 8.4 Validation gates (V40-V43)

| # | Property | Checked against |
|---|---|---|
| V40 | EW modifier | No jammers gives a factor of exactly 1 and unchanged detection - EW-off is §3 exactly; a jammer cuts detection monotonically in power and proximity |
| V41 | Tiger problem | `bayes_update` reproduces the exact posteriors of the standard POMDP toy problem: 0.85 after one observation, 0.9698 after two, with symmetric reversal |
| V42 | Belief well-formed | Belief stays a normalised non-negative distribution; a peaked detection likelihood concentrates it and lowers entropy |
| V43 | Negative information | Repeatedly *not* detecting shifts belief out of coverage and into dead ground; the motion model raises entropy |

---
## 9. Air: drones and counter-air

Air assets are a **third class** alongside units and sensors: airframes that fly at a chosen
altitude, heading and speed along a flight path or a transit-then-orbit plan. Strike drones bomb
ground units; an **air-defence** class answers them, with an engagement model – and therefore a
time-to-kill distribution – that varies by type. A drone may instead, or also, carry a sensor,
making it a mobile elevated observer. Lives in `airframes.rs`, `air_defence.rs` and
`sim/counter_air/`.

The operational-research content is the **sensor-to-shooter timeline**: air defence can be
self-cueing or forced onto external cueing across a comms link with a configurable latency, so
raid leakage becomes a measurable function of cue delay, magazine depth and engagement
channels. That falls straight out of the §3 sensing model.

### 9.1 Altitude, actor height, and the slant-range convention

**Altitude is per instance**, with a reference frame, because the two behaviours differ in
exactly the way that matters: whether terrain can mask the airframe:

$$
h(p) = a \quad \text{(AGL)}
\qquad\qquad
h(p) = \max\big(0,\ a - z(p)\big) \quad \text{(AMSL)}
$$

where $a$ is `altitude_m`, the case is chosen by `altitude_ref`, and $z(p)$ is the ground
elevation under the airframe.

$h$ is precisely the **actor height** of §1.2, so line of sight, viewshed and sensing need no
change whatsoever: `line_of_sight(terrain, a, h_a, b, h_b)` already takes arbitrary endpoint
heights. An AGL drone hugs the terrain and is never masked by the hill it overflies; an AMSL
drone cruises level and *is* masked by higher ground. The $\max(0, \cdot)$ clamp means an AMSL
altitude below the local ground is a drone on the deck: degenerate but well-defined, never
negative.

**Slant range replaces horizontal range**, project-wide:

$$
r_{\text{slant}}(a, h_a, b, h_b) =
\sqrt{ \lVert b - a \rVert^2 + \big((z(b) + h_b) - (z(a) + h_a)\big)^2 }
$$

used for the detection cutoff and falloff $f(r)$ of §3.2 and for both weapon range gates of §2.

On flat ground with equal endpoint heights the height difference is zero, so it reduces
**exactly** to the old horizontal range and the §3 and §4 gates were unchanged by construction.
Every previously measured result was already a slant-range result. That is why adopting it
needed no re-baselining at all.

*Alternative considered:* slant range only when an endpoint is airborne. This keeps
ground-against-ground bit-identical, at the price of two range rules that disagree by design.
Rejected: one convention, documented.

**Terrain effects on an airborne target.** Concealment and cover are properties of the cell a
target *stands in*, and an airborne target is not in one. Consequently, an air target contributes
$\text{concealment} = 0$ to the §3.2 rate and $\text{cover} = 0$ to damage. Canopy
transmittance $\tau$ is *not* waived: it is a property of the sightline, so a low drone seen
through a belt of woods is attenuated exactly as §1.4 says.

### 9.2 Flight kinematics

State: `pos`, `altitude_m` with `altitude_ref`, `heading_deg`, `speed_m_s`. Pure and RNG-free -
flight is deterministic, and all air stochasticity lives in detection and engagement.

A **flight plan** is a waypoint list plus a terminal, which covers both requested behaviours
with one structure:

```
FlightPlan { waypoints: [Vec2], terminal: Hold | Orbit { radius_m, clockwise } }
```

"Fly this path" is `terminal = Hold`; "go here and orbit at radius R" is a single waypoint with
`terminal = Orbit`. The orbit centre is always the final waypoint.

**Transit.** Advance $\text{speed} \cdot dt$ along the polyline, reusing the §6.1 route logic.
The desired heading is the bearing to the next waypoint; the actual heading turns toward it at
up to $\text{max\_turn\_rate\_deg\_s} \cdot dt$. A turn-rate limit implies a minimum turn radius

$$
r_{\min} = \frac{v}{\omega_{\max}}
$$

with $\omega$ in radians per second: the closed form V47 gates, via the chord of a turn through
angle $\phi$ being $2 r_{\min} \sin(\phi/2)$.

**Orbit.** On reaching the terminal waypoint the airframe captures the circle at its nearest
point, then integrates the phase directly:

$$
\theta(t + dt) = \theta(t) \pm \frac{v}{R} dt,
\qquad \text{pos} = c + R(\cos\theta, \sin\theta)
$$

with heading tangent to the circle. Integrating the phase rather than steering toward the
circle keeps the radius exact – no drift over a long loiter – and gives the closed-form lap
time $T = 2\pi R / v$.

**Endurance.** `endurance_s > 0` removes the airframe once its time aloft exceeds it.

### 9.3 Strike

A strike drone's aim point is its assigned target – a named asset or a fixed point – or, if
none was assigned, its **final waypoint**. On closing within `release_range_m` (slant) of the
aim point it releases one munition, which is **exactly the §2.3 indirect round**: burst point
$b = \text{aim} + N(0, \sigma^2 I)$ with $\sigma = \text{cep\_m}/1.1774$, Carleton damage
$D(\rho)$, delivered as $D \cdot (1 - \text{cover})$.

One generalisation of §2.3 is needed. An indirect round damages only the unit it was aimed at,
but a strike on a *point* must damage whatever is near the burst. Damage is therefore applied
to **every live ground unit within $3 R_L$ of the burst**: beyond $3 R_L$ the Carleton kernel
is below $1.2 \times 10^{-4}$, a documented cutoff that keeps the sweep $O(\text{units})$. Each
surviving element rolls independently as in §4.1, and near misses feed §4.3 suppression
unchanged.

`munitions` counts releases and `expendable` decides whether the airframe survives its attack.
Together they span the modern spectrum: a reusable guided-bomb carrier is
`munitions = 2, expendable = false`, and a one-way attack munition is
`munitions = 1, expendable = true`. The same model, different dials.

**Autonomous targeting.** `autonomous` on the airframe's stat block lets a drone release on an
enemy **its own side has located** when it has no live assignment, instead of flying past it.
Off by default, which is every airframe written before it existed. Three constraints, each
deliberate:

- The target must be **located**. Autonomy here means acting on the side's picture without
  waiting to be told, not seeing through terrain.
- It must already be inside `release_range_m`. The drone does **not** divert, so this is
  opportunism along the route it was given. Diverting couples targeting to path planning, which
  is a larger and separate question.
- The choice is ranked by the **same** value function and doctrine tier the ground shooters
  use (§13), so a side ordered to kill command posts first does it with drones too.

Precedence is assignment, then autonomy, then the flight plan's destination. An explicit
assignment wins outright, so opportunity never overrides orders.

*Measured* on `scenarios/hunter_uas.toml`, 500 paired seeds: a drone given no target destroys
**0.000** enemy elements without autonomy and **9.154 ± 0.074** with it ($t = 123.3$, no seed
tied). Without the decision it flies the route, releases at its last waypoint and hits nothing.

Blinding the observer takes it straight back to 0.000. **Autonomy is worth exactly nothing
without the sensor that finds the targets**: the drone may choose, and has nothing to choose
from. Worth stating plainly, because it is easy to read autonomy as a property of the airframe
when it is really a property of the side's picture.

One case is subtler than it looks. A **named** target that no longer exists yields no release
for a non-autonomous airframe – not a fallback to the destination – because that is the identity
half of V60. An autonomous airframe in the same position looks for something else, which is
re-targeting rather than a fallback.

### 9.4 Air defence: two engagement models, two closed forms

Two models, because **time-to-kill is distributed differently** in each: differing in shape and
not just in mean, so guns and missiles fail differently against a saturating raid. That is the
whole reason to model both rather than tune one "effectiveness" number.

**Gun or CIWS (Close-In Weapon System): a Poisson kill process.** While the target sits in the
envelope, kills arrive at rate $\lambda_k$, so per tick $p = 1 - e^{-\lambda_k dt}$. This is
structurally identical to the §3.2 glimpse model, so it inherits its tick-size invariance and its
validation machinery:

$$
\text{TTK} \sim \mathrm{Exp}(\lambda_k), \qquad
\mathbb{E}[\text{TTK}] = \frac{1}{\lambda_k}, \qquad
P(\text{kill by } t) = 1 - e^{-\lambda_k t}
$$

**Missile: discrete shoot-look-shoot.** A launch takes
$t_f = r_{\text{slant}} / \text{missile\_speed}$
to arrive, then resolves as a Bernoulli trial with single-shot kill
probability $p$; a miss is followed by $t_r$ reload before the next launch. Shots-to-kill is
Geometric($p$), and the time to the $N$-th arrival is $N t_f + (N-1) t_r$, so

$$
\mathbb{E}[\text{shots}] = \frac{1}{p}, \qquad
\mathbb{E}[\text{TTK}] = \frac{t_f}{p} + \left(\frac{1}{p} - 1\right) t_r
$$

*Alternative considered for the missile:* modelling interception kinematically, with the missile
as a pursuing body and the kill depending on closing geometry. Rejected: the interesting
variable here is delay and magazine, not endgame guidance, and $p$ and $t_f$ are the dials that
carry the behaviour.

**Envelope.** An engagement requires the target inside `[min_range_m, max_range_m]` in slant range
*and* inside `[min_alt_m, max_alt_m]` – the altitude band is what separates a low-tier CIWS from a
high-tier SAM (Surface-to-Air Missile) – plus line of sight if `requires_los`, a free engagement
channel, and magazine remaining. `channels`, the number of simultaneous engagements, is the
saturation lever a raid plays against.

**One asymmetry between the closed form and the simulation, stated rather than reconciled.**
`resolve_due` charges $t_r$ after a **miss** only, so a battery that kills cleanly may relaunch
on the next tick. That reads as the reload being the *look* in shoot-look-shoot – you only
re-engage a target you failed to kill – and it is self-consistent. It is not what the closed form
assumes: $\mathbb{E}[\text{TTK}]$, `shot_opportunities` and therefore the §11.2 allocation payoff
all price a $t_f + t_r$ cycle per shot regardless of outcome. The payoff is thus very slightly
pessimistic about a battery's throughput against a stream of targets. The two answer different
questions: the closed form is about killing *one* target, the simulation about what a battery
does next.

### 9.5 The cueing timeline

A battery acts on whichever route to the track reaches it first: its own radar, or the network:

```
actionable_at = min( own_sensor_seen,                  // organic: no comms hop
                     first_detected + cue_latency_s )  // handed over the net
              + reaction_time_s
```

`own_sensor_seen` is when **this battery's** organic sensor first saw the target, and is
unavailable if it has no sensor, if its per-instance `self_cue` switch is off, if its radar is
not `emitting`, or if it simply has not seen the target yet. Turning `self_cue` off forces the
asset onto the external cueing chain and makes it pay `cue_latency_s`. The two switches are
distinct: `self_cue` is *whose track the battery acts on*, `emitting` is *whether its radar is
running at all* (§12.5).

**Taking the minimum is what makes this exact rather than approximate.** Every airframe records
when each sensor first saw it, in `AirState.seen_by`, so a self-cueing battery whose radar
acquires the target *after* someone else's sensor detected it still engages off its own radar
instead of waiting out a comms hop it never needed. The consequence is that the air detection
loop runs each sensor's glimpse process until **that sensor** has seen the target, rather than
stopping at the first global detection.

This yields the phase's headline closed form, and it turns on **what the clock starts on**: the
cueing chain begins at *detection*, not at envelope entry.

Let a drone be detected $D$ seconds before it enters the envelope – its **warning lead** – and
spend $W$ seconds inside the envelope before reaching its release point. The battery is
actionable from $t_{\text{entry}} - D + L + R$, so the effective engagement window is

$$
W_{\text{eff}} = \max\big(0,\ W - \max(0,\ L + R - D)\big)
$$

The delay costs nothing until $L + R$ outruns $D$: a cue that has already aged through the
network while the drone was still inbound arrives ready. Consequently the **critical latency**,
beyond which every drone leaks however lethal the battery is, is

$$
L^* = W + D - R
$$

and early warning raises it one second per second. **Early-warning range and comms latency trade
directly against each other**, which is the operationally interesting statement this section
exists to produce.

Leakage then follows from §9.4: for a gun $P(\text{leak}) = \exp(-\lambda_k W_{\text{eff}})$;
for a missile with $K = \lfloor (W_{\text{eff}} - t_f)/(t_f + t_r) \rfloor + 1$ shot
opportunities, zero if $W_{\text{eff}} < t_f$, $P(\text{leak}) = (1-p)^K$.

The $D = 0$ case – acquired exactly as it enters the envelope – gives the simpler
$W_{\text{eff}} = W - L - R$ and $L^* = W - R$. That special case is what the `air_raid`
experiment isolates by setting the radar's range equal to the gun's.

Which of these dials actually decides raid leakage was settled by global sensitivity analysis
rather than argued: cruise speed dominates cue latency. §14.3 has the worked result.

### 9.6 Determinism and the air-off identity

Air adds phases to the loop. They are **appended, and draw zero RNG values when the air and
air-defence lists are empty**, so a drone-free scenario reproduces the pre-air event log
bit-for-bit: the identity discipline of §7.4. The air phases are 2, 4, 6 and 7 in §7.2.

A recce drone's sensor needs no phase of its own: **every sensor lives in one list**, and a
carried one reports the position, height and facing of the airframe carrying it, through
`Sim::sensor_view`. For an uncarried sensor that resolves to exactly its own position and
`mount_height_m`, so phase 3 is unchanged – same draws, same order – whenever there is no air.
Phases 4, 6 and 7 iterate empty lists and draw nothing at all.

A carried sensor's public `pos` and `facing_deg` are **written back from the airframe each
tick**, immediately after air movement. The airframe stays the source of truth and `sensor_view`
is still the accessor that knows about altitude, but leaving the public fields frozen at the
placement point made every consumer outside the detection loop – the app's coverage and belief
overlays, the diagnostic probes – plot a recce drone's sensor at its take-off position and
ground mount height.

`Sim::sensor_active` is the matching gate: a carried sensor dies with its airframe, so a
shot-down drone drops out of coverage rasters as well as out of the detection loop. §12.2 later
extended the same function to know that a radar dies with its battery.

**Ground fires cannot accidentally engage air**: target selection iterates the *unit* list, so
the separation is structural rather than a gate that could be forgotten.
`WeaponType.engages_air`, default false, exists as the opt-in seam for a future dual-role gun and
changes nothing today.

### 9.7 Deliberate limitations

- **Autonomy does not divert.** An autonomous drone releases on what comes inside release range
  along its route; it does not re-plan its flight to go and find a target.
- **No air-to-air.** Drones do not engage other drones.
- **Acoustic detection of drones** is the natural modality and remains unimplemented. §3.1's
  `Modality` tag is the seam.
- **Emissions are binary.** A radar is on or off; there is no intermittent emission and no
  blinking to reduce exposure (§12.5).

### 9.8 Validation gates (V44-V52)

| # | Property | Checked against |
|---|---|---|
| V44 | Altitude and masking | An AMSL drone below a ridge crest is masked from a ground sensor while the same drone at AGL is not; visibility is monotone in altitude, extending V8 |
| V45 | Slant range | Equals $\sqrt{\text{horizontal}^2 + \Delta z^2}$ exactly; a drone at altitude $A$ directly overhead has range $A$, not 0; reduces to horizontal range when $\Delta z = 0$, so V14-V18 stand unchanged |
| V46 | Orbit kinematics | Orbiting at radius $R$ and speed $v$ holds the radius to within $\varepsilon$ and closes a lap in $2\pi R/v$, to within one tick |
| V47 | Transit and turn rate | Straight-leg travel is $\text{speed} \cdot t$, mirroring V37; under a turn-rate limit the achieved turn radius is at least $v/\omega_{\max}$ |
| V48 | Gun time-to-kill | Monte Carlo mean TTK is $1/\lambda_k$ and $P(\text{kill by } t) = 1 - e^{-\lambda_k t}$ within binomial CI - the §9.4 exponential law, gated as V14 and V15 are |
| V49 | Missile time-to-kill | Per-shot kill fraction equals `ssk_p` within binomial CI; $\mathbb{E}[\text{shots}] = 1/p$; $\mathbb{E}[\text{TTK}] = t_f/p + (1/p - 1)t_r$ |
| V50 | Cue latency and leakage | Leakage rises monotonically in `cue_latency_s`, matches $\exp(-\lambda_k W_{\text{eff}})$, and reaches 1 above the critical latency; warning lead $D$ raises $L^* = W + D - R$ one second per second, and $D = 0$ reproduces $W - L - R$ |
| V51 | Envelope and magazine gating | Exactly zero engagements outside the slant-range band, outside the altitude band, without line of sight when `requires_los`, without a cue when `self_cue` is off, or with an empty magazine; concurrent engagements never exceed `channels` |
| V52 | Air-off identity | No air and no air defence gives an event log bit-identical to the pre-air build; with air, the same (scenario, seed) reproduces exactly |

V52's zero-draw half is the one gate that lives inside `sim_core` rather than in the validation
crate, because it asserts a property of the RNG draw stream itself: genuinely internal state
that a public-API test cannot reach.

---
## 10. The decision layer

Everything up to here modelled the world without anyone *deciding* anything. Fires picked the
nearest enemy, routes were drawn by hand, sensors stared where they were placed, and the belief
filter of §8.2 was computed for a display that no simulation code read.

This section closes the loop **sensing → belief → decision → action**: tracks decay (§10.1),
fire is allocated side-wide (§10.2), sensors point themselves by belief (§10.3), and units plan
their own routes against the live risk raster (§10.5). Lives in `allocation.rs`,
`sim/tasking.rs` and `sim/planning.rs`.

### 10.1 Track lifecycle

§3.2 made detection permanent, with track loss deferred to EW. That deferral had teeth: a unit
once seen stays seen forever, so **jamming a tracked unit did nothing at all**. EW could prevent
a track but never break one, which is half the model missing rather than a simplification.

Detection is now derived from a last-observation time. `UnitState` and `AirState` carry
`last_seen_s: Option<f64>`, and at each epoch

$$
\text{detected} \iff \text{now} - \text{last\_seen\_s} < \text{track\_hold\_s}
$$

`detected` stays the field everything else reads, so indirect-fire gating (§4.2) and the §9.5
cueing timeline needed no change at all. Air keeps its per-sensor `seen_by` record as well,
because §9.5 needs to know *which* battery saw the target.

A lapsed air track clears the whole cueing record, `detected_at_s`, `detected_by` and `seen_by`
- not just the flag. Otherwise reacquisition would find a stale `detected_at_s` already aged
past `cue_latency_s + reaction_time_s`, and a battery would fire the instant the target
reappeared, skipping the §9.5 timeline entirely.

**Maintenance runs at the decision epoch, not the tick.** The glimpse loop skips already-detected
targets, so refreshing a track means looking again: measured at 4 sensors by 6 units by 97 µs,
about 2.3 ms per tick, up to twenty times the whole tick budget. At a 10 s epoch that amortises
to 0.23 ms per tick. The cadence is right on its own terms too: tracks decay over tens of
seconds, and maintaining one is a decision-layer concern.

**Maintenance is deterministic, not a fresh glimpse.** Acquisition stays stochastic; keeping eyes
on something already found is not a coin flip. A track refreshes when the sensor's *effective*
rate $\lambda_{\text{eff}}$: the §8.1 jammed rate, with concealment, range and canopy folded in
- clears a threshold:

$$
\text{refresh} \iff 1 - e^{-\lambda_{\text{eff}} \Delta t_{\text{epoch}}} \ge p_{\text{maintain}}
$$

Using the effective rate rather than bare geometry is what lets EW break a track: a jammer that
drives $\lambda_{\text{eff}}$ below `track_maintain_p` ages the track out even with clean line of
sight. A pure "can it still be seen?" test would have re-opened the exact gap this closes.
Drawing nothing also leaves the per-tick RNG stream unperturbed, which is what keeps V52 and V24
from moving.

### 10.2 Fire allocation

Replaces the nearest-enemy rule with a side-wide assignment, solved once per epoch per side
before anyone shoots. For shooter $i$ and slot $k$ of target $j$,

$$
\text{payoff}\big[i\big]\big[(j,k)\big] = q(i,j)\cdot \text{value}(j)\cdot \big(1 - \bar{q}(j)\big)^{k}
$$

**$q(i,j)$ is the fraction of the target destroyed this epoch**, from the existing fires model -
`direct_p_hit` or `expected_area_damage`, times cover, suppression factor and round count,
exactly as a round resolves: clamped to $[0,1]$. Ineligible pairings (out of range, no line of
sight, undetected for indirect) are forbidden outright.

**$\text{value}(j)$** is $\text{elements} \times \text{per\_element}$, where `per_element` is the
optional `value` dial on the stat block, or, when absent,

$$
1 + \frac{\theta}{\theta_{\max}},
\qquad \theta = \text{rof} \times p_{\text{kill|hit}} \times \text{max\_range}
$$

Therefore, an unscored stat block still ranks sensibly – a unit is worth its size, doubled if it is the
most dangerous thing on the field – and doctrine ("kill the radar first") can be stated when
wanted. Per *element*, so a half-destroyed unit is correctly worth less.

**$q$ is an expectation, clamped, not a probability.** For direct fire it is
$\text{rounds} \cdot p_{\text{kill}} / \text{elements}$; for indirect,
$\text{rounds} \cdot \mathbb{E}[\text{damage per round}]$. Both are linear in the round count
rather than the exact $1 - (1-q)^{\text{rounds}}$, and both are clamped. That is fine for
*ordering* pairings, which is all the assignment needs, but it has a consequence worth naming
because the same number is reused as $\bar q$ in the slot discount: once $\text{rounds} \cdot q$
reaches 1 the clamp bites, $(1 - \bar q)^k$ collapses to zero for every slot past the first, and
the diminishing return becomes a cliff rather than a curve. Scenarios where one shooter can
expect to destroy a whole target in one epoch are therefore the ones where the discount does
least work.

**Slots and the discount.** Every target offers a slot to every free shooter, and slot $k$ is
discounted by $(1 - \bar q)^k$: the $(k+1)$-th shooter only helps if the $k$ before it all
failed. This is the standard weapon-target-assignment decomposition, exact when the shooters on a
target are alike. Turning diminishing returns into extra columns is what keeps the problem a
plain **linear** assignment rather than a submodular one.

There is deliberately **no hard cap** on top of it for ground fires: §11.4 says why, and V68
holds the replacement.

$\bar q$ is averaged over the shooters that *could* engage the target, not over those actually
assigned to it, which is the only thing available before the problem is solved. The bias has a
direction: a distant shooter that will never be chosen drags $\bar q$ down, which under-discounts
the later slots and so mildly *encourages* piling on. Exact when the shooters are alike; worth
re-checking with a `sim.allocation` sweep on a scenario with deliberately heterogeneous shooters.

Solved by Hungarian (Kuhn-Munkres) over shooters by slots, with `greedy` and `independent` – the
old per-shooter rule – alongside. `[sim] allocation` chooses.

**Forbidden pairings are scored zero, not $-\infty$.** Kuhn-Munkres produces a *perfect*
matching, while what is wanted is a maximum-weight matching that may leave a shooter idle. With
non-negative payoffs and rows no more than columns, any partial matching extends to a perfect one
using only zero-weight cells without changing its total, so the two optima coincide and
assignments landing on a forbidden cell are dropped afterwards.

A large negative sentinel would have been worse than merely wrong. `1e18 + 10.0` **is** `1e18` in
`f64`, so every matching with the same number of forbidden cells would have scored identically -
a precision trap this codebase has fallen into once and now avoids by construction. §13.2 records
where the same trap was avoided a second time.

**Measured: coordination pays, optimality does not.** On `scenarios/fire_allocation.toml` – four
shooters that can all reach all four targets – over 2,000 paired seeds:

| Rule | Time to destroy Red | Paired against `independent` |
|---|---|---|
| `independent` (the pre-decision-layer rule) | 75.4 ± 0.2 s | baseline |
| `greedy` | 62.5 ± 0.2 s | **−12.84 ± 0.22 s**, significant |
| `optimal` | 62.9 ± 0.2 s | **−12.43 ± 0.23 s**, significant |

**Coordinating is worth about 17%**, unambiguously, and the mechanism is the spread: the old rule
sent every gun at the nearest target while three others stood untouched.

However, **solving the assignment optimally is measurably worse than greedy**: by **0.405 ± 0.051 s**
($t = 8.0$) when the two are compared directly against each other rather than each against
`independent`. They agree outright on 96% of seeds; the cost is concentrated in the 4% where they
diverge. §10.6 traces that to the bottom, and it is not a defect in the solver.

> Measured *directly against each other*, never by eye across a shared baseline. The two
> baselines do not combine the way a paired comparison does, and reading a difference across
> them overstates its error roughly fivefold, which is how this effect stayed invisible for two
> phases. §14.2 states the rule this produced.

On the other shipped scenarios the difference is exactly zero on every seed: with one or two
shooters that can each reach one enemy, all three rules agree. Allocation only matters when there
is a real choice to make, which is why `fire_allocation.toml` had to be built for this experiment
to have anything to measure at all.

### 10.3 Belief-driven sensor tasking

The simulation gains a per-side `SpatialBelief` on a coarse grid (`[sim] belief_cells`, default
48), updated each epoch from what the side's sensors *failed* to see and then diffused by
`predict`. This is the point at which the §8.2 POMDP (Partially Observable Markov Decision
Process) layer stops being a display and starts driving the simulation.

**The objective is information gain, computed exactly.** For a candidate facing the observation
is binary per cell: either a sensor detects something at cell $c$, with probability $b(c)p(c)$,
collapsing belief to a point mass of zero entropy; or it sees nothing and belief becomes
$b'(c) \propto b(c)(1 - p(c))$. So

$$
\mathbb{E}\big[H_{\text{after}}\big] = \left(1 - \sum_c b(c) p(c)\right) H(b')
$$

$$
\text{gain}(\text{facing}) = H(b) - \mathbb{E}\big[H_{\text{after}}\big]
$$

and each steerable sensor takes the facing maximising the gain. Sensors with no `for_width_deg`
see all round and have nothing to choose.

**Why it is affordable.** The expensive part of a detection rate is the line-of-sight walk, and
**line of sight does not depend on facing**: only the field-of-regard gate does. Therefore, the per-cell
rate is computed once per sensor with the arc removed, cached against the pose it was built for,
and each of the twelve candidate facings is then a cheap arc mask over that raster. Without this,
one epoch would cost a viewshed per facing per sensor.

**Carried sensors, and the cache key that pays for them.** A drone-mounted sensor moves every
tick, so an exact pose key would rebuild its raster every epoch and never hit. Carried sensors
were therefore excluded from the belief layer at first, which was the wrong call, and instructive
about why: *not finding anything is evidence*, negative information is the whole point of the
POMDP layer, and the most mobile observer on the field was the one asset excluded from it. A recce
drone could fly the length of the map and leave its side's belief unchanged.

They are now included, keyed on a pose **quantised to the coarse belief grid**, and to a 25 m
altitude band. This is not a fudge: the raster *is* a coarse-grid object – every entry is a rate
at a coarse cell centre – so keying it on the coarse cell the sensor stands in is consistent with
the resolution the whole layer runs at. The cost becomes proportional to how far the drone has
flown rather than to how long it has been airborne, and quantisation is integer arithmetic, so
the rebuild schedule is identical on every run.

Emplaced sensors keep their **exact** pose as the key. They do not move, so the cache hits every
epoch after the first and there is nothing to buy by approximating, and V57 stays pinned to the
real geometry.

A carried sensor still has nothing to *steer*: it faces where its airframe points, and
`sync_carried_sensors` would overwrite any choice made here on the next tick. As a result, it contributes
coverage without participating in the facing decision.

**Off by default (`[sim] sensor_tasking`).** A `facing_deg` written in a scenario is a statement
of intent, and silently overriding it would change what every existing scenario means. It would
also dissolve the §6.3 interdiction game, whose Blue strategies *are* committed postures: a
sensor that re-points itself is no longer playing a strategy. V39 caught exactly this when the
default was briefly `true`, which is the gate doing its job: it caught a model change that would
otherwise have quietly invalidated a whole phase.

*Measured*, on `scenarios/sensor_search.toml`: three 70°-arc observers, five Red units, none of
them in the sectors the observers start on: a fixed stare finds **2 of 5**; the belief-driven
sweep finds **5 of 5**. Nothing about the sweep is scripted. Each sensor drains its own belief out
of ground it has cleared, so the best-information facing moves on by itself.

### 10.4 Validation gates (V54-V58, V61)

| # | Property | Checked against |
|---|---|---|
| V54 | Removal preserves history | Removal tombstones rather than shifting, so every index already in an event log still resolves to the same asset |
| V55 | Track lifecycle and EW | A track lapses `track_hold_s` after its last observation and is cleared; continuous observation refreshes it indefinitely; jamming drives $\lambda_{\text{eff}}$ below the maintenance threshold and so *breaks* a track, which permanent detection made impossible |
| V56 | Allocation optimality | Hungarian matches an exhaustive brute-force optimum for small problems; its total payoff is never below greedy's; no shooter is committed to two targets in one epoch; an ineligible pairing is never chosen; allocation draws no randomness |
| V57 | Tasking beats staring | Against an enemy hidden outside its initial arc, a belief-tasked sensor detects where a fixed stare never does, with a shorter mean time to detect; belief stays a normalised non-negative distribution with finite entropy across many updates, extending V42; tasking draws no randomness |
| V58 | Decision-layer identity | With one shooter and one reachable target, every allocation rule and both tasking settings produce identical detection and fire logs - the decision phases draw zero randomness, so the stream cannot shift |
| V61 | Carried sensors inform belief | A recce drone that overflies ground and finds nothing drains its side's belief out of that ground, against a control with no drone; belief stays normalised; an emplaced-only scenario is unchanged, because only carried poses are quantised; the cleared ground moves with the drone, so the raster is genuinely refreshed |

**Regression risk, as predicted and as found.** Allocation changes what units shoot at, so V24,
V30 (Lanchester), V31 and V39 were flagged in advance as able to move. V24, V30 and V31 did not,
exactly as reasoned: they are single-shooter or homogeneous-line scenarios where allocation
degenerates to the old choice. **V39 did move**, and not because of allocation but because of
tasking, which is how `sensor_tasking` came to default to off.

### 10.5 Movement decisions in the loop

Fires are allocated and sensors are tasked, but movement stayed scripted for a long time:
`movement::least_risk_path` was called only from `experiments` and `validation`, so the
dynamic-programming strand sat *beside* the model rather than inside it.

A unit now declares **either** a `route` – scripted, exactly as before – **or** an `objective`,
which it plans its own way to, re-solving each decision epoch against the live risk raster.
Declaring both is a load error in the §7.6 family: neither "plan then ignore the plan" nor
"follow the route then re-plan" is obviously the one meant.

**Per unit, deliberately, not a `[sim]` switch.** Two things follow. The identity holds by
*construction*: a scenario with no objective builds no planner and computes no raster, so there
is nothing to switch off (V72). Moreover, a scripted unit and a planning unit can share one map on one
seed, which makes control and treatment a single trial rather than two runs that have to be
trusted to differ in only one way.

**The decision grid is coarse, and the number is what forced it.** A risk raster at full terrain
resolution costs about **4 s** for a 1000 by 1000 map with two sensors, because every cell asks
every sensor for a detection rate and each of those walks a sightline. At one epoch per 10 s that
is a hundred times the cost of everything else, and a 500-seed study becomes a fortnight. So
planning happens on the same coarse grid §10.3 uses for belief, for the same reason: a commander
choosing an approach every ten seconds is not choosing between adjacent 10 m cells. The unit still
*moves* continuously at full resolution.

The coarse edge cost is `TerrainGrid::move_cost`'s own formula – distance times mean mobility
times slope factor – evaluated on cell **aggregates** rather than point samples, so it is an
approximation of the real cost rather than a different cost that resembles it. A coarse cell is
impassable only when *every* fine cell in it is: at 150 m resolution, "there is a way through" is
the honest reading.

`least_risk_path` was split so the planner runs the **same** search: `least_cost_path` takes an
edge-cost function, and `least_risk_path` is that with the terrain's own. V25, V26 and V27
constrain the shared core, so their guarantees reach the in-loop planner rather than a second
implementation needing its own gates.

**Hysteresis, named in advance.** A unit re-deciding every epoch flips between two near-equal
routes as costs wobble: the movement analogue of §13.4's target-lock problem, and it gets the
same answer. A new route is adopted only if it beats the held one by `repath_margin`, default
10%, with the held route re-costed on the *current* raster so a route only looks worse when the
risk has actually moved.

**Fire in the risk raster.** `[sim] fire_risk_weight`, default 0, adds a second term: ground
within an enemy weapon's reach is priced as well as ground an enemy sensor can see. At zero the
raster is enemy observation only, exactly as §5.2 defined it. Direct fire additionally requires a
sightline; indirect requires only range, because its eligibility is a track rather than a line
(§4.2). V77 separates the two facts completely by using a fixture with an indirect gun covering a
corridor and *no enemy sensor anywhere*, so observation risk is identically zero and only the new
term can move the route.

### 10.6 The planning horizon, and what the optimal-versus-greedy gap is made of

`[sim] allocation_horizon` is how many decision epochs the objective prices. A target's value
becomes

$$
v(t) = \text{elements}(t)\left(1 + H\,\frac{\theta(t)}{\bar\theta}\right)
$$

where $\theta$ is the target's threat and $\bar\theta$ the most dangerous unit on the field. Size
counts once; **threat counts once per epoch the target would still be firing**. At $H = 1$ this is
the §10.2 formula exactly, so the default is an identity rather than a re-baseline (V75). A
declared `value` is left alone: that dial is the author saying what an asset is worth outright,
and a horizon second-guessing it would make an explicit statement mean something else.

**What it prices, and what it does not.** The horizon changes *which* target to shoot: it makes a
dangerous one worth more than a merely large one, because the harm a survivor does grows with the
time it has to do it in. On `default` a horizon of 8 stops shooting the air-defence battery
altogether – its threat to ground forces is zero – and concentrates on armed units.

It does **not** close the gap between the optimal solver and the greedy one, at any setting tried:

| Horizon | `optimal` against `greedy` on `fire_allocation` |
|---|---|
| 1 | +0.450 ± 0.076 s ($t = 6.0$) |
| 4 | +0.450 ± 0.076 s |
| 8 | +0.450 ± 0.076 s |

Necessarily so. `fire_allocation` fields four **identical** targets that cannot shoot back, so
scaling every value by the same factor leaves every assignment untouched and a survivor threatens
nothing. Nothing about the future of a *target* can matter there.

**What the gap is actually made of.** Tracing the seeds where the two solvers disagree settles it.
Over 150 seeds, optimal is worse on 7, **better on none**, and identical on 143, and on the seeds
where it loses, both solvers fire the *same number of rounds*. Nothing is being wasted. The
divergence is always at the endgame:

```
[14] greedy  0 -> Unit(7)      optimal  0 -> Unit(6)
[15] greedy  1 -> Unit(6)      optimal  2 -> Unit(6)
[16] greedy  2 -> Unit(6)      optimal  3 -> Unit(7)
[17] greedy  3 -> Unit(7)      optimal  2 -> Unit(7)   <- one epoch later
```

Greedy pairs its last shooters so that both remaining targets die in the same epoch. The optimal
assignment scores higher on expected damage and leaves one target barely alive, costing a whole
epoch.

**The objective maximises expected damage; the outcome is a completion time.** Expected damage is
indifferent to how damage is spread across survivors. Time-to-clear is not: it depends only on
the last target standing. Throughput against makespan, in the usual scheduling sense.

**Why no better value function fixes it.** The obvious repair is to make finishing a target worth
more than wounding two. That was implemented as a flat completion bonus and then removed, because
it did nothing measurable at any setting. The reason is structural, and it is the useful part of
this result: **both solvers optimise the same objective**, so improving the objective moves both
together, and the gap is not a property of the value function at all. Greedy does not win because
it values completion: it has no notion of completion. It wins because its heuristic bias happens
to produce assignments that finish targets, and exact maximisation of an additive surrogate does
not.

Closing it properly would mean optimising the thing that actually matters: the probability that
*every* remaining target dies this epoch. That is a **product** over targets, not a sum, so it is
not a linear assignment problem and Kuhn-Munkres cannot express it. An additive objective over
slots can never encode a makespan.

Consequently, the honest position is that V56 holds, the solver is correct, the objective is a reasonable
surrogate, and **the surrogate's structure – not its coefficients – is what costs the 0.45 s**.
Optimising a surrogate harder does not improve what the surrogate stands for.

### 10.7 Deliberate limitations

- **The allocation objective scores a single epoch.** This is the largest open limitation in the
  model, and §10.6 measures exactly what it costs.
- **The planned route is quantised to the decision grid.** Waypoints are coarse-cell centres, so
  a unit deviates up to about half a cell from the ideal line even at `risk_weight = 0` -
  measured at about 170 m on a 7 km map at 48 cells. It is the cost of planning affordably, and
  it is why V73 measures deviation in *hundreds* of metres rather than asserting a straight line.
- **The mover is a reference, not the unit.** Risk is a property of the ground and the enemy's
  sensors, computed once per side, so an unusually stealthy unit is not routed differently from a
  conspicuous one.
- **A route choice is a step, not a curve, on a map with one detour.** Demonstrated by
  `scenarios/covered_approach.toml`: caution below 800 takes the direct corridor, 800 and above
  takes the detour, and further caution buys nothing because there is nothing left to buy.
  Tracing a real frontier needs a map with graded alternatives.
- **Belief is over enemy position and never folds in a detection** (§8.3). The tasking objective's
  derivation assumes a belief over an *undetected* enemy, and the two readings have not been
  reconciled.

### 10.8 Deferred

**A completion-time objective**, maximising the probability that every remaining target dies this
epoch rather than the expected damage dealt. §10.6 establishes that this – not the value
function's coefficients – is what the optimal-versus-greedy gap is made of. It is a product over
targets rather than a sum, so it leaves the linear-assignment family entirely and needs a
different solver, which is why it is deferred rather than attempted.

### 10.9 Validation gates (V72-V77)

| # | Property | Checked against |
|---|---|---|
| V72 | Movement decisions are a structural identity | A scenario in which no unit declares an `objective` reproduces exactly and its scripted units follow their routes to the metre - no planner is built and no risk raster computed. Declaring both `route` and `objective` on one unit is refused at load, naming the unit; the same exclusivity holds when they are set interactively |
| V73 | A planned route avoids what watches it | With an enemy sensor squarely on the straight line to its objective, a planning unit leaves that line by hundreds of metres; with `risk_weight = 0` - same map, same sensor, same planner - it goes straight. That is V25's zero-risk-is-the-shortest-path arriving inside the loop, and the second half is what makes the first mean something |
| V74 | A planner does not dither | A watcher on the line makes passing north and passing south cost almost exactly the same, which is what makes a fresh solve wobble. The committed direction must not flip and the unit must arrive. Deliberately *not* monotone progress toward the objective: a detour increases straight-line distance before it decreases it, so that test would forbid routing around anything |
| V75 | The allocation horizon | $H = 1$ reproduces the single-epoch objective exactly, event for event, because it multiplies the threat term by 1.0. Above 1 the fire plan genuinely changes, and the formula ordering is checked arithmetically: a large harmless target must be overtaken by a small dangerous one as the horizon grows |
| V76 | Autonomous strike targeting | An airframe without `autonomous` aims only where it was told or at its flight plan's destination. With it, a drone passing a located enemy aims at that enemy instead, ranked by the same value function and doctrine tier the guns use. An explicit assignment still wins. Checked by where munitions were *aimed*, not how many were dropped |
| V77 | Fire in the movement risk raster | At `fire_risk_weight = 0` the raster is enemy observation only. Above zero, ground within an enemy weapon's reach is priced too, and raising the weight never reduces the clearance taken. The fixture separates the two facts completely: an indirect gun covering a corridor and no enemy sensor anywhere |

---
## 11. Command and control

Ground fires coordinate side-wide for free (§10.2): defensible for a battlegroup sharing one
fire-control net. Air defence should not, and did not: each battery independently engaged
whatever was nearest.

**Coordination is an asset you field, not a switch you set.** A dial would have made "the
batteries cooperate" free and permanent. Making it a placed **C2 post** makes it something that
must be paid for, positioned, and can be taken away, which is the behaviour worth modelling, and
the seam SEAD (Suppression of Enemy Air Defences) hangs off. Lives in `c2.rs` and
`sim/counter_air/coordinate.rs`.

### 11.1 The C2 post

A post is a placed asset with a `coordination_range_m`. Batteries within that radius of a **live
friendly** post allocate as one group; batteries outside act on their own. It has no weapon, no
sensor, and does not move. Its only effect is on who is in whose assignment problem.

**Range is horizontal, not slant** (§9.1). A coordination link is a communications relationship,
not a sightline; using slant range would make a post on a hill mysteriously worse at talking to
the battery beneath it.

Destroying a post costs no battery, no magazine and no envelope. What is lost is the
coordination: from the next tick the group **decoheres** and every battery reverts to
nearest-first. That is what makes "kill the command post" a better opening move than "kill one
more launcher", and the model produces it without being told to.

**The link is not all-or-nothing.** Two things bear on it besides the post being alive.

*Jamming pulls the radius in.* The effective coordination range is

$$
r_{\text{eff}} = r_{\text{coord}} \cdot g(\text{post})
$$

where $g$ is `ew::jamming_factor` evaluated at the post. Therefore, an enemy jammer near the post does
not flip the link off: it shrinks it, and the batteries on the flanks fall out of the net while
the one sitting on top of the post keeps talking. That is the right shape: a link degrades with
range against a noise floor, and raising the floor is what a jammer does. It also gives the raid
a **soft** counter beside SEAD's hard one: same effect on the defence, no ordnance spent, and
nothing on the map to show it happened.

Note the sign, which is the mirror of §8.1. `Sim::jamming_at` folds a side's **own** jammers,
because a jammer protecting Red degrades Blue's sensing of Red. `Sim::link_quality_at` folds the
**enemy's**, because a jammer degrades Blue's own communications. Same asset, same dials,
opposite side of the argument, so one Red jammer both hides Red units and cuts the Blue net.

*Joining costs time.* `link_latency_s`, default 0, is how long a battery must have been inside
the radius before it is in the net. Defaulting to zero recovers the pre-latency behaviour
exactly, and – the reason it matters for study design – lets a sweep turn either effect on
**alone**, without the other confounding it. The consequence worth noting is that a battery not
yet in the net falls back to nearest-first and commits its channel: a link that arrives late
cannot retrospectively undo the duplicated engagements already made.

### 11.2 The air-defence payoff

`max_batteries_per_air_target`, default 2, is the only overkill cap left in the model. The ground
side has none: §11.4 says why.

An airframe is different from a ground unit, and that is why the dial survives here. It is a
**single object**, so a second battery is insurance against the first missing rather than extra
damage delivered, and a missile is a discrete round out of a finite magazine rather than a
continuous rate. There is a real quantity to be wasted, which is what a cap is for.

**Rows of the assignment are free engagement channels, not batteries**, so a two-channel battery
contributes two rows and `channels` falls out of the structure rather than needing a special
case. Columns are slots on each engageable airframe, discounted geometrically as in §10.2:

$$
\text{payoff}\big[\text{channel}\big]\big[(\text{air}, k)\big] =
P(\text{kill before release}) \cdot \text{value}(\text{air}) \cdot (1 - p)^{k}
$$

**The deadline is the release point, not the envelope edge.** A drone that leaves the envelope
having already dropped its munition has won, so the window is the time to reach `release_range_m`
of its aim point, and the battery best placed to stop the airframe *closest to doing damage* wins
it, not the one that happens to be nearest. An airframe with nothing left to drop has no such
deadline and is scored over the time to cross the envelope instead: still worth shooting, just
not urgently.

The window is capped at a **planning horizon of 60 s**, for two reasons, the second concrete.
Beyond about a minute, "how long this target will linger" stops discriminating usefully, since
the defence will have reconsidered many times. Further to this, an uncapped window runs to hundreds of seconds
for a distant loiterer, which drives $P(\text{kill})$ to 1 for *every* pairing; the
diminishing-return discount $(1-p)^k$ then collapses to 1 and stops separating "cover another
drone" from "pile onto this one", which is the entire job it is there to do. That degeneracy was
observed while building V59, not theorised.

One consequence reads as counter-intuitive and is worth stating plainly rather than hiding: a
bomber seconds from release has a *short* window and therefore a *low*
$P(\text{kill before release})$, so it scores below a recce drone the battery can
comfortably catch. That is the
formulation being self-consistent, not a bug: maximising expected value destroyed says shoot
what you can still stop, and a bomber past the point of interception is a lost cause. Whether it
is the *right* objective is a separate question; making `value` reflect imminent harm, rather
than only what an airframe carries, is the natural way to change the answer.

$P(\text{kill} \mid \text{window})$ is `air_defence::p_kill_in_window`, the same §9.4 pair of laws
V48 and V49 gate – exponential for a gun, geometric for a missile – evaluated forward over a
window rather than sampled, so the two cannot drift apart. $\text{value}(\text{air})$ is the
optional `value` dial on `AirType`, or a derivation from remaining munitions and whether the
airframe carries a sensor, so an unscored stat block still ranks a loaded bomber above a spent
one.

**One assignment per side.** A post coordinates its own batteries and nobody else's, so when both
sides are coordinated there are two problems, not one. This is not a refinement: solving them
together has no well-defined answer, because a pooled group has no single doctrine to be scored
under and no single `max_batteries_per_air_target` budget to spend. V59 fields a mirrored
two-sided engagement and requires each side to make the same choice it would make with the
enemy's post removed.

**Measured: coordination is about ammunition, not kills.** On `scenarios/ad_c2.toml` – three SAM
batteries against a tight packet of ten drones – over 500 paired seeds:

| | Downed (of 10) | Rounds left (of 24) | Leakers |
|---|---|---|---|
| No C2 | 9.33 ± 0.04 | 0.82 ± 0.06 | 0.77 ± 0.02 |
| With C2 | 9.92 ± 0.01 | **3.65 ± 0.11** | 0.69 ± 0.02 |
| Paired difference | +0.59 ± 0.04 ($t = 15$) | **+2.83 ± 0.11 ($t = 27$)** | −0.08 ± 0.02 ($t = -4$) |

The kill count barely moves. What moves is the **magazine**: the coordinated defence ends with
four and a half times the reserve, having achieved slightly more. Uncoordinated, it very nearly
shot itself dry against a raid it was otherwise winning, and a defence out of rounds is a defence
that loses the next raid.

**Why this scenario uses missiles, and why that is the whole point.** A gun is a Poisson process,
so two batteries on one target simply add their kill rates, $\lambda + \lambda$, and nothing is
lost: **stacking guns is not wasteful**. A missile launch is a discrete round out of a finite
magazine, so three interceptors at a drone one would have killed is two rounds that will not be
there for the next one. **Coordination pays exactly where the shot is a countable resource**,
which is a sharper statement than "coordination is good", and it falls out of the two engagement
models rather than being asserted.

**Measured: the cap itself is not earning its keep.** The reasoning for
`max_batteries_per_air_target` above was still a claim, so it was swept: 1,000 paired trials on
`ad_c2`, against a cap of 1:

| Cap | Drones downed | Rounds left |
|---|---|---|
| 2 | +0.013 ± 0.011 ($t = 1.2$) - **not significant** | −0.233 ± 0.064 ($t = -3.6$) |
| 3 | −0.020 ± 0.012 ($t = -1.6$) - **not significant** | −0.584 ± 0.067 ($t = -8.7$) |

On this scenario the second battery buys **nothing measurable** and costs about a quarter of a
round; the third buys nothing and costs more than half a round. Neither is a disaster and neither
is an argument for the default: it is defensible but unearned here. Whether a cap above 1 earns
its keep when batteries are scarce relative to the raid is the open question, and it is one this
scenario cannot answer, because it has three batteries and ten drones.

### 11.3 Ground fires and the net

§10.2 let a side coordinate its ground fires for free while §11.1 made air defence pay for a post.
That asymmetry was reasoned rather than arbitrary – a battlegroup does share one fire-control net,
where point-defence batteries genuinely are independent sites – but it was an *argument*, and an
argument is not something you can measure.

`[sim] fires_need_c2` makes it a modelled thing. With it on, `Sim::allocate_side` splits a side in
two and solves **two** problems:

| Shooter | Solver |
|---|---|
| inside a live friendly post's (jammed) radius | the scenario's `allocation` - the side-wide assignment |
| outside it | `independent` - picks for itself, the pre-decision-layer rule |

**Two separate problems rather than one problem with constraints, deliberately.** "Not in the net"
means precisely "does not know what anyone else is doing", so an unnetted shooter must not be
allowed to *avoid* a target because a netted one took it. Solving them together would leak exactly
that information.

Being outside the net costs coordination, not the ability to fight: a loose gun still engages, on
its own judgement. What it loses is shown by V63's three-gun case: it opens on a target one of
the netted guns has already destroyed, and its whole volley leaves no trace in the log. That
wasted volley is what the net buys back.

**Off by default.** Turning it on unconditionally would silently reduce every existing scenario to
`independent`, re-baselining the §10.2 allocation result, V56 and V39 at once, for a reason
invisible in the scenario files. As a dial, the cost of losing the net is a number instead.

### 11.4 Ground fires have no overkill cap

A target's slots are priced by the $(1 - \bar q)^k$ discount alone (§10.2), with no hard limit on
how many shooters may be assigned to it.

There was a cap, `max_shooters_per_target`, and it is worth recording why it went, because the
lesson is transferable. A multi-element ground unit genuinely absorbs several shooters, and a hard
cap *idles* the surplus: a target offering $\min(\text{elements}, \text{cap})$ slots leaves
shooters beyond that assigned nothing and firing nothing, which is a worse error than overkill
whenever there is nothing else to shoot. §10.2 already prices piling on; truncating that as well
said "rather than overkill, do nothing".

Worse, the cap was applied **per fire-control problem rather than per side**, so turning on
`fires_need_c2` applied it twice and a *divided* side fought measurably better than a coordinated
one. **A dial whose meaning depends on the scope it is applied over will invert when that scope
changes.**

Removing it improved the coordinated baseline on `fires_c2` from 103.40 s to **66.80 s** – that
gap is the fire the cap was throwing away – and coordination is now worth about −2.2 s
($t = -2.1$), in the direction it should be. V68 holds the replacement.

Air defence keeps its cap (§11.2) because an airframe is a single object and a missile is a
discrete round, so there is a real quantity to be wasted.

### 11.5 Deliberate limitations

- **The link ignores terrain.** Jamming and latency bear on it, but a ridge between the post and a
  battery does not. A terrain-aware comms model is the natural next refinement, and the §9.5
  cue-latency machinery is the seam.
- **A post cannot be handed off.** There is no notion of a deputy taking over, so killing the only
  post decoheres the defence permanently rather than for a reorganisation delay.
- **A post has no capacity.** It coordinates any number of batteries within its radius equally
  well.

### 11.6 Validation gates (V59, V62, V63, V68)

| # | Property | Checked against |
|---|---|---|
| V59 | C2-coordinated air defence | With one drone nearest to every battery, nearest-first sends them all at it while a post makes them cover one drone each; a scenario with **no** post is unchanged from the pre-C2 engine, so V50-V52 cannot move; a **dead** post coordinates nothing, costing no battery, magazine or envelope - only the coordination; and a post coordinates its **own side only**, so with both sides coordinated each battery still makes the choice it would make with the enemy's post removed |
| V62 | The link degrades, not only dies | An enemy jammer on the post scales its radius by the EW factor, so the flanking batteries drop out and the defence decoheres with nothing destroyed; a *friendly* jammer does not cut its own net; a **zero-power** jammer runs the whole arithmetic and changes nothing (§7.4); `link_latency_s` delays joining, and a battery not yet in the net commits its channel nearest-first, so a late link cannot undo it |
| V63 | Ground fires can be made to need C2 | With `fires_need_c2` on, two guns under a live post engage two targets while the same guns without one both take the nearest; with the dial off the fire log is bit-identical; the net is per shooter, so a post covering two of three guns leaves the third choosing for itself; and jamming scales the post's radius, so a link pulled to 20 m drops a gun 40 m away out of the net |
| V68 | The overkill discount replaces the overkill cap | With three guns and one target all three engage, where the old hard cap assigned nothing to the surplus and they fired nothing at all; with a target each they still take one each, so the geometric discount delivers the spread the cap was credited with |

---

## 12. SEAD: air defence as a target

§9.7 listed "air-defence sites are not attritable" as a deliberate limitation, and §11 then made
it awkward: the model claimed a command post was the thing worth killing first, while providing no
way to kill one from inside the simulation. This section closes that. Lives across
`sim/counter_air/`, `sim/engagement.rs` and `air_defence.rs`.

### 12.1 What changed

**Batteries and posts have elements.** `AirDefenceType.element_count` and `C2Type.element_count`,
both defaulting to 1, give them the same sub-element attrition as a unit (§4.1), so a near miss
degrades a battery rather than only ever destroying or sparing it. `alive()` is `elements > 0` on
both.

**Any ground asset can be named as a target.** `TargetSpec::Named(id)` resolves against units,
then air-defence batteries, then C2 posts. Ids are unique within a scenario, so one namespace
covers all three and SEAD needs no new syntax: `target = { unit = "sam-1" }` simply works, with
`asset` as the clearer alias.

**Area damage sweeps them.** The §2.3 Carleton kernel applies to batteries and posts exactly as to
units, with terrain cover, rolled per surviving element. Nothing about the mathematics cared which
list an asset lived in; only the sweep did.

### 12.2 What death costs

The interesting part is not the destruction but its consequences, and the two differ:

| Asset destroyed | Firepower lost | Second-order effect |
|---|---|---|
| **Battery** | its launchers | **its organic radar goes dark** - an emitter the rest of the network was cueing from (§9.5) |
| **C2 post** | **none at all** | the group it coordinated **decoheres** and reverts to nearest-first (§11.1) |

A destroyed battery also drops its open engagements, so its channels are not left occupied by a
corpse.

**The radar consequence is what makes SEAD worth more than the launchers it removes**, and it
falls out of existing structure rather than needing a special case: an organic radar is an
ordinary entry in the sensor list, and `Sim::sensor_active` – which already knew a carried sensor
dies with its airframe (§9.6) – now also knows a radar dies with its battery. Coverage and belief
rasters drop it automatically, because they were already asking that question.

### 12.3 Anti-radiation homing: the radar buys its own accuracy

§12.2 made a battery's radar the thing worth killing. This makes it the thing that makes killing
it *possible*.

A real anti-radiation missile rides the radar's own signal down, so its accuracy is bought with
the target's emissions. Two dials on the weapon say so:

```toml
[weapons.arm]
anti_radiation = true
cep_m          = 5.0     # against a transmitting radar
silent_cep_m   = 400.0   # against a silent one
```

`WeaponType::cep_against(emitting)` is the single place that decides, and for anything without the
flag it returns `cep_m` whatever the emitter is doing: a dumb shell's accuracy does not depend on
what its target is transmitting. Therefore, every existing munition is an exact identity (§7.4), and an
ARM (Anti-Radiation Missile) with no `silent_cep_m` stated falls back to `cep_m`, meaning
declaring the flag alone changes nothing until the degradation is given a number.

**A dispersion, not a veto.** The munition still arrives; with nothing to home on it flies to
where the emitter was last known to be. "An ARM cannot engage a silent radar at all" is this with
the value set very large: reachable as a scenario's choice, rather than baked in as the model's
opinion. The veto version would also flatter the counter, since switching a radar off would become
a free and total defence.

**What counts as an emitter.** Only a *named*, live, `emitting` battery with a working organic
radar. A command post, a unit or a bare map point radiates nothing an ARM could ride, so an ARM
sent at one is flying blind by definition rather than by omission.

**The trade this poses, and why it is a real one.** `emitting = false` is the counter, and it is
not free: the radar is **off**, so the battery detects nothing through it, cues nothing with it
and contributes no coverage. It can still be handed a track by some *other* sensor over the net –
that is what `self_cue` governs, and the two are separate flags for exactly this reason – but a
battery whose radar was its only sensor is simply blind. As a result, the defender chooses: **survive the
missile, or see the raid coming.** Not both.

Measured on `scenarios/sead_arm.toml`, 500 paired seeds: EMCON (Emission Control) takes batteries
killed from 0.980 to 0.10, and in the same breath takes detections from 0.996 to **0.000**, shots
to 0.000 and drones downed to 0.000.

A tuning note worth keeping, because it generalises: the carrier first released at 1500 m, inside
a defending gun's 2000 m envelope, and was shot down before releasing on 99.6% of seeds. Both arms
of the comparison read 0.00 and the seeker was never exercised at all. **Release range and
engagement envelope are a matched pair**, and a scenario that measures nothing is worse than no
scenario, because it looks like evidence.

### 12.4 Ground counter-battery

Air-delivered SEAD was the only kind, because ground fires iterated the unit list. Every asset
class already had elements and already took §2.3 area damage identically, so the only thing
missing was *which lists are searched*. `FireTarget` now names the list, and `TargetState` gathers
the four facts a shell depends on – where, how big, how many left, is it locatable – so
counter-battery arrived by widening a list rather than by writing a second fires model.

Units come first in `engageable_targets`, with batteries and posts appended, so a scenario with no
enemy emplacements produces exactly the list it always did (§7.4).

**How an emplacement is found is the interesting part.** Neither batteries nor posts go through
the §3.2 glimpse loop, so neither has a track, and indirect fire needs one. Inventing a stochastic
acquisition here would have inserted draws into every scenario fielding air defence and shifted
the stream under V50, V51, V59 and V60 for no modelling gain. So `Sim::emplacement_is_located`
asks the question counter-battery acquisition actually asks: **has it given itself away?**

| Asset | Located when |
|---|---|
| Battery | it is transmitting (`emitting` with a live radar) **or** it has fired |
| Post | it is coordinating at least one live battery |

Those are the two real ways a site is fixed: electronic support measures on its emissions, or a
counter-battery track back along its rounds. A command post is found because it is *talking*: the
same argument in a different band. All three are deterministic and draw no randomness.

This joins the two halves of §12.3. Switching a radar off already made an ARM miss; it now also
hides the battery from artillery. **One decision, three consequences**, and the cost stays what
§12.3 said it was.

**Direct fire is unchanged:** line of sight and range, no track. Consequently, going silent hides a battery
from the guns behind the hill, not from the tank looking at it.

**Value has no derivation across classes.** A unit's is
$\text{elements} \times (1 + \theta/\theta_{\max})$ with threat
$\text{rof} \times \text{lethality} \times \text{reach}$. A battery's danger is to *aircraft* and
a post has no firepower at all, so neither has an output measurable on that scale, and a
conversion would be arithmetic dressed as doctrine. Both fall back to 1.0 per element, and a
scenario that wants artillery to prefer the SAM over the tanks says so with `value`. That is what
the dial is for: "kill the radar first" is a judgement, not a derivation.

### 12.5 Emission and cueing are two decisions

`emitting` and `self_cue` are separate flags, and the separation is load-bearing.

- **`emitting`** is whether the radar transmits. False means it detects nothing, cannot cue its
  own battery, contributes no coverage, and gives a seeker nothing to ride.
- **`self_cue`** is *whose track the battery acts on*. The radar runs either way; the battery
  either acts on its own track or waits out `cue_latency_s` for one over the net (§9.5).

They are independent because a battery may act on its own radar while under emission control, or
wait for a track over the net while transmitting. Collapsing them into one flag lets a battery
take the missile protection of going dark **while still detecting everything**: survivability
without the blindness that is supposed to pay for it.

Re-measured with the flags separate, 500 paired seeds on `sead_arm`: a battery under EMCON
survives the missile – 0.10 against 0.980 killed, a paired difference of **−0.884 ± 0.015** – and
in exchange its detections fall from 0.996 to **0.000**, taking its shots and its kills with them.
That is the trade this section claims, and the model delivers it.

### 12.6 Deliberate limitations

- **An ARM still homes where it was sent.** An `autonomous` airframe picks its own target from
  what the side has located (§9.3), but an anti-radiation seeker rides the emitter it was aimed at
  rather than scanning for the nearest transmitting radar. Choosing a target and homing on one are
  different mechanisms, and only the first is autonomous.
- **Emissions are binary.** A radar is on or off; there is no intermittent emission, no blinking
  to reduce exposure, and no memory of a position after the emitter goes quiet beyond the aim
  point itself.
- **Counter-battery acquisition never expires.** A battery that fired one round at $t = 5$ s is
  still located at $t = 600$ s, with no re-acquisition. A *unit* seen once is not: §10.1 lapses
  its track after `track_hold_s`. **The two acquisition models disagree about how long knowledge
  lasts, and nothing yet says why.** This is an open question rather than settled model, and a
  result that leans on long-lived counter-battery knowledge should say so.
- **"Has fired" records a resolution, not a trigger pull.** For a missile battery those coincide.
  For a gun, `resolve_due` logs only a tick that *killed*, so a gun that has been firing steadily
  and hitting nothing has not "fired" for this purpose, which is not quite what §12.4 claims.

### 12.7 Validation gates (V60, V64, V65, V69)

| # | Property | Checked against |
|---|---|---|
| V60 | Air defence is attritable | A strike drone assigned a named C2 post destroys it **in simulation** and the defence decoheres with no battery lost; a destroyed battery's organic radar stops emitting; a target id matching nothing yields no aim point, so the new asset lists are additive rather than a replacement |
| V64 | Anti-radiation homing | The same missile lands with `cep_m` against a transmitting radar and `silent_cep_m` against a silent one, the mean miss scaling as the ratio of the two CEPs ($\mathbb{E}\lvert\text{miss}\rvert = \sigma\sqrt{\pi/2}$, $\sigma = \text{CEP}/\sqrt{2\ln 2}$) and doing correspondingly less damage; a weapon without the flag ignores the emitter entirely, and an ARM with no `silent_cep_m` falls back to `cep_m` - both exact identities; only a named, live, self-cueing battery counts as an emitter |
| V65 | Ground counter-battery | A howitzer kills an emitting SAM and the fire log names what it hit; a battery that has neither transmitted nor fired is **not located**, so indirect fire has nothing to aim at; direct fire needs no track, so a silent battery in plain view is still a target; a post is located while it is coordinating and stops being so when it has nothing left to coordinate; a scenario with no enemy emplacements is unchanged |
| V69 | Emission and cueing are separate decisions | A battery with `emitting = false` records zero detections, cannot cue itself, and gives an ARM nothing to home on; a battery with `self_cue = false` keeps its radar running and detects exactly as much as one that does not, differing only in whether it may act on its own track. Both defaults are true, so an ordinary battery is unchanged |

---
## 13. The kill chain: directed targeting

§10.2 allocates fire by maximising $P(\text{kill}) \times \text{value}$. That is what an
**omniscient optimiser** would do, and as a bound on how well a side could possibly shoot it is
exactly the right thing to compute.

It is not how a force fights. A gun crew does not hold a kill-probability table. It holds
**orders** – engage air defence before manoeuvre, shoot the command post first, counter-battery
takes precedence – and it follows them whether or not the shot in front of it is a good one.

Therefore, a declared priority here is **strict by default**: a shooter that can reach anything in a
higher tier takes it, even at a worse kill probability than a lower tier offers. That is not a
crude approximation of the optimiser; it is a different decision rule, and for a directed force
a more faithful one.

**Which turns the mode switch into a measurement rather than a preference.** Running the same
scenario under strict doctrine and under the payoff-optimal allocation puts a number on *what
directive control costs against optimal control*. Compare with §10.2, which measured what *no*
control costs against optimal: the three together bracket the question. Lives in `doctrine.rs`.

### 13.1 What a priority entry may name

Four things, checked in this order, all equally valid:

| Entry | Matches |
|---|---|
| an asset **id** | that one asset - how a gate pins an exact target |
| a **role** | every asset whose stat block declares it (`role = "artillery"`) |
| a **class** | `unit`, `air_defence`, `c2`, `air` - always available, never declared |
| `"all"` | anything - the tier that says "and then everyone else, equally" |

**There is no "no doctrine".** A side always has one; omitting the block gives
`priority = ["all"]`, a single tier holding every target and ranked among itself by the ordinary
payoff, which *is* the undirected behaviour. Therefore, the engine has **one** code path rather than
two, and the identity with the pre-doctrine model holds by construction rather than by a
separate branch that has to be kept honest. `"all"` is usable mid-list too, which makes the
bottom tier explicit: `["c2", "air_defence", "all"]` reads as the fire plan it is.

Roles are free-form strings on the stat block, so a scenario can invent whatever categories it
needs; adding "engineer" takes no code change. **A role never masks its class** – a battery with
`role = "sam"` matches both `"sam"` and `"air_defence"` – so a coarse doctrine keeps working when
a stat block later becomes more specific.

The **first** matching entry decides the tier, which is what lets a list single one asset out and
then name the class beneath it: `["sam-1", "air_defence"]` puts that launcher a tier above its own
siblings. Anything unnamed falls into an implicit bottom tier, still ranked among itself by the
ordinary payoff.

**Every name is checked when the simulation is built.** A priority entry matching nothing is a
load error listing what would have worked, not an empty tier. Same reasoning as the schema's
`deny_unknown_fields` (§7.6): a tier that silently matches nothing fails invisibly: the run
succeeds and simply answers a different question from the one asked.

### 13.2 How a priority is applied

| Mode | Rule |
|---|---|
| `strict` (default) | tier decides; the payoff only breaks ties *within* a tier |
| `weighted` | tier scales the target's value by $\text{falloff}^{-k}$; the payoff still decides |

**Strict is implemented by solving the assignment one tier at a time**, highest first. Any shooter
that can reach a tier takes something in it; only those left unassigned fall through to the next.
That makes the ordering *exact*.

The obvious alternative – add a large bonus to a higher tier's payoff – is a trap this codebase
has already met once. `allocation::INELIGIBLE` was originally $-10^{18}$, and at that magnitude
`1e18 + 10.0 == 1e18` in `f64`, so every matching with the same forbidden-cell count scored
identically (§10.2). A tier bonus large enough to dominate would swallow the payoff differences
*inside* a tier in exactly the same way. A sequence of small exact problems has no such failure
mode.

Doctrine applies to **ground fires and air defence** alike, and on both the coordinated and the
uncoordinated paths. Being outside the C2 net (§11.3) costs a battery its coordination, not its
orders: a lone gun still shoots what it was told to shoot first, it simply does not know what
anyone else is shooting.

### 13.3 Ordered engagements

```toml
[[blue.orders]]
shooter = "gun-a"
target  = "sam-1"
```

The bluntest instrument here, and the one a gate usually wants. An ordered shooter is removed from
the assignment problem entirely, so "gun-a engages sam-1" is a *fact about the run* rather than a
likely outcome of it. Everything not under orders is allocated normally, so a scenario can pin one
pairing and let the rest be solved.

Both ends must still be alive; an order against a destroyed target lapses and the shooter rejoins
the problem, because a standing order does not make a crew fire at a wreck.

### 13.4 Eligibility blocks, and a shooter holds what it takes

Two rules keep a fire plan from becoming a way to waste ammunition.

**Line of sight and range block a pairing; they do not merely lower its score.** A target a
shooter cannot engage is `INELIGIBLE` in the payoff and is never returned by any solver, so a
shooter whose whole top tier is masked by a ridge finds nothing there and **falls through to the
next tier**. It is never left idle facing a hill while something it could engage goes unengaged.

`Sim::can_engage` is the one test – alive, in range, in line of sight for direct fire, holding a
live track for indirect – and doctrine, target locks and ordered engagements all ask it, so all
three agree about what "reachable" means. That includes `[[orders]]`: an order stands while the
pairing is reachable and lapses while it is not, resuming the moment the target reappears. Making
orders the exception – "an order is an order, the crew tries anyway" – is defensible in isolation
but contradicts the rule everything else follows, and it would let one bad order silence a gun
indefinitely.

Note that "reachable" is asymmetric between the fire classes, exactly as §4.2 says: a masked
target blocks a direct-fire gun and does not block the howitzer behind it, which needs only a
track that somebody on its side holds. V70 pins both directions.

**A shooter holds its target.** `UnitState.engaging` is a **lock**: once taken, it is kept until
the target is dead or can no longer be engaged. Air defence has always worked this way, through
`AirDefenceState::engagements` and `drop_engagements`; this is the ground half of the same idea.

Without it a gun re-decides from scratch every epoch and flip-flops between two near-identical
targets as tiny payoff differences wobble: wasted fire for a reason no crew would recognise.
Switching targets is itself a decision with a cost, so it should take something changing on the
ground, not a rounding difference. The one thing that *does* break a lock is a new order, which is
the point of an order. §10.5's `repath_margin` is the same idea applied to movement.

**A held lock still counts against its target's discount.** Otherwise the $(1-\bar q)^k$ sequence
would restart for every shooter that happened to be re-deciding, and a target already covered by
three locked guns would look as attractive to a fourth as an untouched one.

**Measured** on `scenarios/kill_chain.toml`, 500 paired seeds over 20 s: armour-first gives 1.42
armour elements destroyed and 0 posts; c2-first gives 0.00 and 0.60 ($t = -21.7$ and $+27.2$).

In addition, an unobvious result worth keeping: **batteries killed comes out lower under the
air-defence-first plan.** A tank must be *found* – first detection around 3.5 s – while an
emitting battery is locatable from tick one (§12.4), so even the armour-first plan opens on the
battery because nothing else is visible yet. **Doctrine ranks what you can see; it cannot rank
what you have not found.**

### 13.5 Deliberate limitations

- **Doctrine is static.** A side's priority does not change with the situation; there is no
  "switch to counter-battery once the air threat is gone". A conditional kill chain is the natural
  next step and would sit on the same tier machinery.
- **Priority is not per shooter.** The whole side shares one list. Giving a specific battery its
  own priority would need doctrine on the instance rather than the force.
- **Doctrine reaches an autonomous drone, but not an assigned one.** An `autonomous` airframe
  ranks what it finds by the same priority list the guns use (§9.3), so a side's orders carry to
  its drones. A drone with an explicit target still attacks that target: an order to a specific
  airframe outranks the side's general priority, which is the same precedence `[[side.orders]]`
  has over `priority`.

### 13.6 Validation gate (V66)

| # | Property | Checked against |
|---|---|---|
| V66 | Directed targeting | A gun with a 46% shot at a near high-value tank and a 3% shot at a far SAM takes the tank unprompted and the **SAM** when told `priority = ["air_defence"]` - strict doctrine is followed, not weighed; the same priority in `weighted` mode does not overturn the fifteen-fold better shot, which is the difference the two modes exist to express; an ordered engagement bypasses the assignment and lapses if its target dies; a side with no doctrine is an exact identity; a priority naming nothing is a load error listing what would have worked, and ids, roles and classes are all accepted; air defence follows the same doctrine; line of sight and range **block** a pairing rather than lowering its score, so a masked priority target does not hold a shooter hostage; and a shooter holds its target until it is dead or unengageable, with a held lock still consuming a slot |

---

## 14. The measurement machinery

A model that is never measured is a model that merely used to be right. This section specifies the
statistics the study harness rests on: what makes a comparison valid, and what the estimators are
checked against. The *operation* of the tools is in [`docs/GUIDE.md`](GUIDE.md); this is the
argument for why they are built as they are. Lives in `crates/experiments/src/`.

### 14.1 Fix the map, vary the dice

`Sim::new(scenario, libs, seed)` derives **both** the terrain and the RNG stream from one seed.
Looping it over seeds therefore varies the map and the luck together, and the result averages two
sources of variance that answer different questions.

Every study builds the terrain **once per worker, from the scenario's own `default_seed`**, and
calls `Sim::reset_to_scenario` between trials. As a result, the map is held fixed and the question is "what
happens on *this* map, on average". Averaging over maps is a fair question, just a different one,
and it is asked by sweeping `default_seed` itself.

This arrived as a performance change – terrain generation is seconds and a trial is microseconds –
and turned out to be a correctness change. That is worth noticing as a pattern: a study that is
too slow to run enough seeds tends to be a study measuring the wrong variance.

### 14.2 Compare paired, always

Two arms of a study run the **same seed set**, so arm A and arm B are matched trial for trial:
same map, same dice, one dial different. The difference is taken seed by seed:

$$
d_k = a_k - b_k, \qquad \mathrm{SE}(\bar d) = \frac{s_d}{\sqrt{n}}
$$

This is **common random numbers**, and it works because

$$
\mathrm{Var}(a - b) = \mathrm{Var}(a) + \mathrm{Var}(b) - 2\,\mathrm{Cov}(a, b)
$$

The two arms share the map and most of the luck, so $\mathrm{Cov}(a,b)$ is large and positive and
most of the variance cancels. Comparing the two *unpaired* means throws that away and can be an
order of magnitude noisier.

`experiments::stats` therefore offers `paired()` and **no unpaired comparison at all**, and
`paired()` panics if the two arms are different lengths rather than quietly comparing what it has.

**Two arms are compared against each other, never each against a shared baseline.** This is the
sharper form of the same rule and it is easy to get wrong, because a table of arms against one
baseline invites reading the difference between two rows. Their separate errors do not combine the
way a paired difference does. On the allocation comparison, greedy and optimal differ by
0.405 ± 0.051 measured directly, but read off their shared `independent` baseline the gap looks
like 0.4 against standard errors of about 0.23: five times noisier, and enough to hide a real
effect entirely.

**The tie count is the most informative field in the report and the one most often skipped.** A
small difference with a *high* tie count means the two arms are mostly making the same decision,
which is a different conclusion from "the effect is real but hard to see". When every arm ties on
every seed, the harness says so explicitly, because "no significant effect" reads as evidence of
no effect when it is usually evidence that the dial does not reach the metric in this scenario at
all.

Errors shrink as $1/\sqrt{n}$: to halve the error bar, quadruple the seeds.

### 14.3 Screening and variance decomposition

Every number in this project is an abstract placeholder, which leaves one question hanging over
every finding: *does it matter that the numbers are invented?* Neither a one-dial sweep nor a
factorial design can answer it, because both vary a few dials with the rest pinned wherever the
scenario happened to leave them. Both measure a slice through a space they never explore.

**Global sensitivity analysis** explores the space and reports what share of the outcome's
variance each dial is responsible for. Two estimators, in the standard two-stage arrangement.

**Morris elementary effects**, first, because it is cheap. $\mu^*$ ranks dials by how much they
move the answer and $\sigma$ flags one whose effect depends on where the others are. Its job is to
say what can be *ignored* before the expensive pass runs. Cost is $(k+1) \times \text{trajectories}$.

**Sobol indices**, second, as a variance decomposition from Saltelli sampling:

| Index | Meaning |
|---|---|
| $S_1$ | the share of variance this dial explains **alone** |
| $S_T$ | the share it is involved in altogether, interactions included |
| $S_T - S_1$ | the share running **through** interactions - invisible to a one-dial sweep |

Cost is $n(k+2)$ design points, which is why screening comes first: at 123 dials a full
decomposition is millions of trials.

The sum of the first-order indices is the diagnostic that matters. Near 1 means the dials are
additive and one-at-a-time sweeps are sound. Well below 1 means most of the variance lives in
interactions, and a sweep will mislead.

A slightly **negative** $S_1$ means "indistinguishable from zero". The Saltelli estimator is
unbiased rather than non-negative, and clamping it would hide how noisy a near-zero index is.

**A worked result**, from `studies/sensing.toml`: what decides whether a drone raid gets through,
over 32,720 trials and four dials:

| Dial | $S_1$ | $S_T$ | $S_T - S_1$ |
|---|---|---|---|
| `air.strike_uas.cruise_speed_m_s` | 0.653 | 0.801 | 0.148 |
| `air_defence.sam.cue_latency_s` | 0.339 | 0.374 | 0.035 |
| `sensors.mast_optical.lambda0_per_s` | −0.001 | 0.057 | 0.058 |
| `sim.track_hold_s` | 0.000 | 0.000 | 0.000 |

**Raid speed dominates**: how fast the attacker crosses the envelope explains more than the
defender's cue latency does, which is not what §9.5's emphasis on the cueing timeline would lead
you to guess. **The sensor barely matters**, which retrospectively explains why sweeping its
glimpse rate over a twentyfold range moved leakage by only 0.176: it is not the binding constraint
here. **`track_hold_s` is exactly inert**, because the engagement resolves faster than the
shortest hold time in the range. Moreover, the first-order total of **0.990** says the dials are
additive on this scenario, which is a licence for every one-at-a-time sweep run on it.

### 14.4 Interactions

A factorial design reports **main effects** first, each averaged over every level of the other
factors – so a factor is described by what it does across the design rather than at one corner of
it – and **interactions** second, as the classic difference of differences, formed per seed:

$$
(y_{11} - y_{01}) - (y_{10} - y_{00})
$$

Zero means the two dials are additive and can be reasoned about separately. Non-zero means they
cannot, and **that decides whether the main effects above may be read on their own**. If two dials
interact, "this one is worth −11 s" is a sentence with a missing clause.

This matters more than it sounds. The investigation that removed the ground overkill cap (§11.4)
needed a two-by-two over the cap and how fast targets were acquired, and the *interaction* was the
dominant effect: the cap mattered enormously when targets were scarce and hardly at all when they
were not. Two main effects would have described neither case.

### 14.5 Findings are re-run, not recorded

A measured finding is a claim about a model at a moment. The model then changes, and unless
something re-runs the claim, a number that was right when written goes on being quoted after it
stopped being true.

`findings.toml` pins each documented claim to the paired comparison that produced it – scenario,
dial, two arms, metric, seeds, the expected difference and a tolerance – and the `findings` binary
re-runs them all.

**The tolerance is not a confidence interval.** The run computes its own standard error. The
tolerance is the author's statement of how far the number may move before the prose around it
stops being true, which is a different and more useful question.

A drift report also lists every document repeating the number, because fixing a stale finding is
mostly a matter of finding all the places it was copied to.

### 14.6 Determinism of a parallel study

Results come back in seed order regardless of how the work was split, and each trial is a fresh
`reset_to_scenario` whose RNG stream depends only on its seed. A parallel study therefore returns
byte-identical numbers to a serial one, and a test pins that, because "the study was parallelised
and the answer changed" is otherwise found out months later, by a confusing result.

The seed list is cut into exactly one chunk per worker thread, and each worker builds one
simulation and resets it between trials. That is one terrain build per thread, paid once and
concurrently, and deliberately not `rayon::map_init`, whose init closure is called an unspecified
number of times.

### 14.7 Deliberate limitations

- **Two-way interactions only.** A factorial design reports every pair of factors; a three-way
  interaction is in the per-cell CSV but not in the report. With more than two levels the reported
  interaction is the corner-to-corner contrast rather than the whole surface.
- **Sensitivity dials are continuous only.** A range is a pair of numbers, so a categorical dial
  like `sim.allocation` has no place in a study file: those belong in a factorial design.
- **A sensitivity study may not vary terrain.** Terrain is built once for the whole design, so a
  terrain dial would ask for a map it does not get.
- **`--seeds N` always means seeds $0..N$**, so two studies at different $N$ share a prefix rather
  than being independent. That is deliberate – it is what makes arms pairable – but it means "run
  1,000 more seeds" is `--seeds 2000`, not a second run.
- **No confidence intervals on quantiles**, only on means.

### 14.8 Validation gate (V71)

| # | Property | Checked against |
|---|---|---|
| V71 | The sensitivity estimator against a closed form | Sobol first-order and total indices from Saltelli sampling match the analytic indices of the Ishigami function $f = \sin x_1 + a \sin^2 x_2 + b x_3^4 \sin x_1$ to within Monte Carlo tolerance. The third input is the one that matters: its first-order index is exactly **zero** - $x_3$ does nothing alone, so a one-dial sweep of it finds nothing - while its total index is large, because it acts entirely through $x_1$. That gap is what the estimator exists to expose. Total indices are never below first-order ones, a structural invariant; and Morris screening, an independent estimator, agrees with Sobol about which dials matter |

A gate on the **measuring instrument** rather than on a model, as V27 is for Dijkstra. An
instrument that cannot recover a known answer cannot be trusted with an unknown one.

---

## 15. How the numbering works

Two schemes run through this project, and both are stable names rather than hierarchies to be
learned.

**`§N.M`** is a section of this document. The numbers are referenced from roughly five hundred
places in the source tree, so **a section keeps its number even when its contents are amended by
later work**. They are never renumbered.

**`V1`-`V77`** is a validation gate: one property checked against a closed form or a documented
invariant. V25 is "zero risk weight gives the shortest path".
[`docs/VALIDATION.md`](VALIDATION.md) holds the full catalogue and what each is checked against.

**Sections are not phases.** Work usually amends existing sections rather than adding one, so the
two part company. The map, for reading the source's history:

| Built | What it added | Where it landed |
|---|---|---|
| 1 | Terrain, derived layers, LOS, viewshed | §1 |
| 2 | Sensing and detection, deliberately ordered *before* fires | §3 |
| 3 | Direct and indirect fires | §2 |
| 4 | Units as elements, suppression, attrition | §4 |
| 5 | Movement as dynamic programming | §5 |
| 6 | The game-theoretic layer | §6 |
| 7 | Visualisation | app only |
| 8 | Electronic warfare and the belief filter | §8 |
| 9 | Air, counter-air, and the slant-range convention | §9, and §1.2 for height |
| 10 | The decision layer: tracks, allocation, tasking | §10 |
| 11 | Command and control | §11 |
| 12 | SEAD, making air defence attritable | §12 |
| 13 | Closing the §11 and §12 gaps | amended §10, §11, §12 |
| 14 | The kill chain: directed targeting | §13 |
| 15 | Closing the open model questions | amended §2, §11.4, §12.5 |
| 16 | Measurement machinery: factorial designs, global sensitivity | §14 |
| 17 | Movement decisions in the loop | §10.5 |

**Sensing was deliberately ordered before fires.** Sensing is the centrepiece of the tool and the
visual heart of it, so landing it earlier meant the interactive loop existed sooner, and it
hardened the line-of-sight interface while changing that interface was still cheap.

---

*Back to [README](../README.md) · [Guide](GUIDE.md) · [Model](MODEL.md) ·
[Reference](REFERENCE.md) · [Validation](VALIDATION.md)*
