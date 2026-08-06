# Fires & Manoeuvre Sim

An operational-research simulation of land warfare direct and indirect fires. A Blue
and a Red force — composed from artillery, manoeuvre units, and sensing assets — fight
over terrain that actually matters: line of sight, cover, concealment, mobility. Sensing
is central. You place sensors and try to detect the enemy before being detected, then
watch fires suppress and attrit manoeuvre.

Built to make OR models tangible and tweakable rather than to ship as a game. It is a
personal research and learning tool, written in Rust with a Bevy front-end.

**All unit, weapon, and sensor numbers are abstract placeholder dials — not real
munition or sensor performance data.** The models are the product; the numbers are knobs.

## Design goals

- **Headless, deterministic core.** The simulation is a pure library (`sim_core`, no
  Bevy): given a scenario and a seed it produces identical results with no UI attached,
  so thousands of batch trials, sweeps, and Monte Carlo runs need no window.
- **The maths is the product.** Every model is formulated and validated against a known
  analytical result or a documented invariant before it is made fast or pretty.
  Correctness is testable; "realism" is not.
- **Data-driven.** Unit, weapon, and sensor stats live in TOML, never hard-coded, so
  they are tweakable at runtime.
- **Composable subsystems.** Terrain, fires, sensing, suppression, movement, and
  decision-making are separate modules with clean interfaces, so electronic warfare
  slotted in as a modifier on the sensing channel rather than a rewrite.

---

# The OR strands

Six bodies of theory, each doing a job the others cannot. This section states each one
in its own symbols, says what it buys, and points at the code and the gate that holds it
honest. `docs/DESIGN.md` carries the per-subsystem depth; this is the argument for why
each tool is the right one.

The simulation loop is **hybrid continuous/discrete**. Between decision epochs the state
integrates continuously; at each epoch the discrete decisions are set — what to shoot,
where to move, where to look. That split is not a convenience, it is the structure:
continuous dynamics are an optimal-control problem, the epoch-to-epoch choices are a
dynamic program, and the two only compose cleanly if they are kept apart.

## 1. Optimal control — the continuous dynamics

State evolves under a control you may choose but not violate:

```
ẋ = f(x, u),        u ∈ U(x)
```

For an airframe the state is `x = (p, ψ, v)` — position, heading, speed — and the
binding constraint is a **turn rate**, not a position:

```
|ψ̇| ≤ ω_max     ⟹     r_min = v / ω_max
```

A rate limit is what separates a flight path from a polyline. Ask a drone to fly a
90° corner and it cannot; it flies an arc of radius `r_min`, arriving late and displaced.
That error is exactly what an air defence's engagement window is made of, so modelling
heading as a state with a bounded derivative is load-bearing rather than decorative.

Orbits integrate the **phase** rather than steering toward the circle:

```
θ(t + Δt) = θ(t) ± (v/R)·Δt,     p = c + R·(cos θ, sin θ)
```

Steering would accumulate radius drift over a long loiter; integrating phase holds
`‖p − c‖ = R` exactly and gives the lap time `T = 2πR/v` in closed form to test against.

*Lives in* [`air.rs`](crates/sim_core/src/air.rs) *·  gates V46, V47*

## 2. Dynamic programming — movement as a least-risk value function

A mover choosing a route is solving Bellman's equation over the terrain grid:

```
J*(x) = min  [ c(x, u) + J*(f(x, u)) ]
      u ∈ U(x)
```

with `x` a cell, `u` one of its eight neighbours, and the edge cost

```
c(x, u) = move_cost(x, u) + w · risk(u)
```

`move_cost` is mobility × slope × distance (∞ for impassable); `risk ∈ [0,1]` is a
supplied exposure raster. Because every cost is non-negative there are no negative
cycles, so Dijkstra returns `J*` exactly — label-setting value iteration, not an
approximation of it. Full value iteration is held in reserve for when risk becomes
time-varying and the one-sweep ordering no longer holds.

The interesting object is `w`. It is an **exchange rate**: the metres of mobility cost
a commander will spend to avoid one unit of exposure. Sweeping `w` from 0 upward traces
the Pareto frontier between arriving quickly and arriving alive, which is a far more
useful answer than any single "optimal" route. Additive costs were chosen over
maximising survival `Π(1 − p_death)` because the logarithm turns that into the same
additive problem anyway, and additive keeps the Dijkstra gate clean.

Risk defaults to **enemy observation coverage** — for each cell, the detection rate a
reference mover would suffer from the best-placed enemy sensor. So "least-risk path"
literally means "the route that stays hardest to see", and the see-without-being-seen
idea becomes navigable.

*Lives in* [`movement.rs`](crates/sim_core/src/movement.rs) *·  gates V25–V27*

## 3. Stochastic processes — detection, fires, suppression, attrition

Four processes, chosen so that each has a closed form to test against.

### Detection is a rate, never a per-tick probability

Sensor `s` detects unit `u` as a Poisson process of rate

```
λ(s,u) = λ₀ · f(r) · σ_m(u) · τ(s,u) · (1 − c(u)),      f(r) = 1 / (1 + (r/r_½)ⁿ)
```

where `σ_m` is signature in the sensor's modality, `τ = exp(−κL)` the canopy
transmittance along the sightline, `c` the target's terrain concealment, and `r` slant
range. The rate is zero when LOS is blocked, range exceeds the cutoff, or the target
falls outside the field of regard.

```
P(detected by t) = 1 − e^{−λt}          per tick:  p = 1 − e^{−λΔt}
```

Modelling a **rate** rather than a per-tick probability is what makes the answer
independent of the tick size. Memorylessness gives the exact identity

```
Π_k e^{−λΔt_k} = e^{−λ Σ Δt_k}
```

for any subdivision of the interval, so halving `dt` cannot change the detection
statistics. A per-tick probability would silently make the physics a function of the
integrator — a class of bug that is very hard to see and very easy to ship. V17 checks
the identity to float tolerance.

### Fires: two Gaussians, two closed forms

Direct fire scatters isotropically about the aim point with `σ(r) = δ·r/1000` for an
angular dispersion `δ` mrad. Deflection and elevation errors are independent, so hitting
a `W × H` silhouette is a product of two one-dimensional Gaussian integrals:

```
P_hit(r) = erf( W / (2σ(r)√2) ) · erf( H / (2σ(r)√2) )
```

Indirect fire samples a burst `b ~ N(aim, σ²I)` with `σ = CEP / √(2 ln 2)` — the
circular-Gaussian CEP identity — and delivers the **Carleton** incapacitation kernel
`D(ρ) = exp(−ρ² / 2R_L²)` at burst-to-target distance `ρ`. Marginalising the kernel over
the burst distribution is a Gaussian convolving a Gaussian, so the expected damage at
aim offset `d` is exact:

```
E[D](d) = R_L² / (σ² + R_L²) · exp( −d² / (2(σ² + R_L²)) )
```

That closed form is the reason for this kernel. A cookie-cutter lethality disc is
simpler and cheaper, but it has no analytical expectation, so the Monte Carlo sampler
would have nothing to be checked against — and an unvalidated sampler is exactly what
this project is trying not to build.

### Suppression is a birth–death chain

Per-unit state `S ∈ {Free, Suppressed, Pinned}`, stepping up on near-misses at rate `β`
and decaying at rate `μ`:

```
Free ⇌ Suppressed ⇌ Pinned          π_k ∝ (β/μ)^k,   k = 0,1,2
```

The birth–death structure hands over its stationary distribution for free, and the mean
time Pinned → Free is `2/μ` — two exponential steps in series. Suppression is a *state*
rather than a scalar multiplier because it gates behaviour discontinuously: Pinned units
neither fire nor move, which is what lets fires shape manoeuvre without killing anybody.

### Attrition against Lanchester

Fire volume scales with a unit's surviving elements, so an aimed-fire duel obeys

```
dA/dt = −βB,   dB/dt = −αA     ⟹     α(A₀² − A²) = β(B₀² − B²)
```

Lanchester's **square law**: combat power goes as the square of numbers under aimed
fire. Reproducing it is the strongest single check on the attrition chain, because it
is an emergent property of the whole loop — element counts, rate of fire, hit
probability, removal — rather than of any one function.

### Air defence: same target, two distributions

```
gun      TTK ~ Exp(λ_k)          E[TTK] = 1/λ_k
missile  shots ~ Geometric(p)    E[TTK] = t_f/p + (1/p − 1)·t_r
```

A gun grinds continuously; a missile is discrete shoot-look-shoot with flight time `t_f`
and reload `t_r`. The distributions differ in shape, not just in mean, so guns and
missiles fail differently against a saturating raid — which is the whole reason to model
both rather than tune one "effectiveness" number.

*Lives in* [`sensing.rs`](crates/sim_core/src/sensing.rs), [`fires.rs`](crates/sim_core/src/fires.rs),
[`suppression.rs`](crates/sim_core/src/suppression.rs), [`air_defence.rs`](crates/sim_core/src/air_defence.rs)
*·  gates V14–V24, V28–V31, V48–V49*

## 4. Game theory — sensing against counter-sensing

Where to put a sensor has no best answer, only a best answer *against a thinking
opponent*. That is a zero-sum matrix game. Blue mixes over positions `x ∈ Δ_m`, Red over
routes `y ∈ Δ_n`, and von Neumann's minimax theorem says the game has a value:

```
v = max min xᵀA y = min max xᵀA y
     x    y          y    x
```

Solved by **fictitious play** — each side repeatedly best-responds to the opponent's
empirical distribution of past play:

```
i* = argmaxᵢ Σⱼ A[i][j]·col_counts[j]
j* = argminⱼ Σᵢ A[i][j]·row_counts[i]
```

The time-averaged strategies converge to equilibrium for zero-sum games (Robinson, 1951)
with no LP dependency, and convergence is self-certifying: the value is bracketed by

```
v_low = minⱼ (xᵀA)ⱼ  ≤  v  ≤  maxᵢ (Ay)ᵢ = v_high
```

and the gap `v_high − v_low` shrinks to zero. You can watch the bracket close.

The payoff `A[b][r]` is expected Red attrition when Red traverses route `r` while Blue
observes and bombards from position `b`, estimated by short headless Monte Carlo
battles. So the matrix is built by the simulation itself, and the equilibrium is a
statement about *this* terrain and *these* sensors — not an abstract game.

*Lives in* [`game.rs`](crates/sim_core/src/game.rs) *·  gates V32–V39*

## 5. Partial observability — belief, and the value of not seeing

Once electronic warfare degrades sensing, the tracked quantity is no longer enemy
position but a **belief** over it: `b_t(s) = P(enemy at s | z_{1:t})`, maintained by the
two standard steps,

```
update   b_t(s)  ∝  P(z_t | s) · b_{t−1}(s)
predict  b⁻_t(s') = Σ_s T(s'|s) · b_{t−1}(s)
```

The load-bearing observation is the **negative** one. Not seeing something is evidence:

```
P(no detection | enemy at s) = exp( −λ(s)·Δt )
```

A cell your sensor covers well has a low likelihood of producing no detection, so
belief drains out of it; dead ground and jammed cells sit near 1 and keep their mass.
Stare at open ground long enough and the belief mass migrates, unprompted, into the
folds and the woodline — which is what a competent staff officer does with the same
information, and it falls straight out of Bayes rather than being scripted.

EW enters the *rate*, not the geometry:

```
λ_eff = λ · Π_j g_j(target),     g_j = 1 − power_j·(1 − d/radius_j)
```

so a jammer raises belief entropy `H(b) = −Σ_s b(s) log b(s)` without moving anything.
With no jammers every `g_j = 1` and the product is exactly 1, so EW-off reduces to the
sensing model bit-for-bit (V40) — an identity, not an approximation.

Entropy is also the control objective for the sensor-tasking layer: point each steerable
sensor at the facing maximising the *expected* reduction in `H(b)`. For a candidate
facing, either something is detected — collapsing belief to a point of zero entropy — or
nothing is, and belief becomes `b'(c) ∝ b(c)(1 − p(c))`, so

```
gain = H(b) − (1 − Σ_c b(c)p(c)) · H(b')
```

That closes the loop **sensing → belief → decision → action**. Measured: three
narrow-arc observers searching for five dispersed units find **2 of 5** staring where
they were placed, and **5 of 5** when tasked by belief — and nothing about the resulting
sweep is scripted. Each sensor drains its own belief out of ground it has cleared, so the
best-information facing moves on by itself.

*Lives in* [`ew.rs`](crates/sim_core/src/ew.rs), [`pomdp.rs`](crates/sim_core/src/pomdp.rs),
[`sim/tasking.rs`](crates/sim_core/src/sim/tasking.rs) *·  gates V40–V43, V57*

## 6. Combinatorial optimisation — who shoots what

Which shooter engages which target is an assignment problem, and solving it side-wide
rather than shooter-by-shooter is the difference between three tanks all firing at the
nearest enemy and three tanks covering three enemies. For shooter `i` and slot `k` of
target `j`,

```
payoff[i][(j,k)] = q(i,j) · value(j) · (1 − q̄(j))^k
```

`q` is the fraction of the target destroyed this epoch, straight from the fires model.
The geometric term is the diminishing return on piling on: the (k+1)-th shooter only
helps if the `k` before it all failed. Expressing it as extra columns is what keeps this
a plain linear assignment — solved optimally by Kuhn–Munkres — instead of a submodular
problem needing a bespoke solver.

A greedy allocator ships alongside, and it earns its place by settling what the optimal
solver is actually worth. On a scenario built to present a real choice, 500 seeds compared
paired: **coordinating is worth −11.3 ± 0.5 s (~15%)** off the time to destroy the enemy,
unambiguously. **Optimality is worth nothing measurable** — greedy and Hungarian differ by
0.12 s against a standard error of ~0.5, and agree outright on most seeds. At this scale
greedy's myopia costs it essentially nothing.

That is the useful kind of negative result, and it only exists because the baseline was
kept. (An earlier version of this claimed greedy *beat* the optimal solver; it did not —
that came from unpaired means with no error bars. The experiment now reports a standard
error on every figure.)

*Lives in* [`allocation.rs`](crates/sim_core/src/allocation.rs) *·  gate V56*

*Lives in* [`ew.rs`](crates/sim_core/src/ew.rs), [`pomdp.rs`](crates/sim_core/src/pomdp.rs)
*·  gates V40–V43*

---

## Layout

| Path | What it is |
|---|---|
| `crates/sim_core/` | Headless, deterministic OR engine — pure Rust, no Bevy. Where all the maths lives |
| `crates/app/` | Bevy front-end: tactical map, pan/zoom, egui control panel |
| `crates/experiments/` | Headless batch runs: sweeps, Monte Carlo, equilibria. Depends on `sim_core` only |
| `crates/validation/` | The V1–V59 gates: every model checked against a closed form or a stated invariant |
| `scenarios/` | TOML scenarios and the unit/weapon/sensor stat blocks |
| **`docs/HOW_IT_WORKS.md`** | **Start here if you are new.** How detection, engagement and scenarios actually work, with worked numbers |
| `docs/DESIGN.md` | The deep spec: equations, state machines, and the validation gate for every model |
| `SETUP.md` | Environment setup (Rust + Bevy + VSCode), written for a Rust beginner |

Inside `sim_core`, `sim/` is the engine that drives everything else: `sim/mod.rs` holds
the tick, with detection, engagement, setup and the air phases in sibling modules. The
model code around it — `sensing.rs`, `fires.rs`, `movement.rs` — is pure functions with no
state, which is what lets the validation crate check each one in isolation.

`sim_core` never depends on `app` or on Bevy. That boundary is what keeps the maths
independently testable and the simulation runnable headless.

## Quick start

```
cargo run -p app              # open the tactical map window (the `default` scenario)
cargo run -p app -- air_raid  # open a named scenario from scenarios/
cargo test --workspace        # run the engine tests and the validation gates
cargo run -p validation --release --bin validation_report   # the V-gate table
cargo clippy --workspace      # lint
```

Any `scenarios/*.toml` that parses as a scenario can be opened by bare name, or by path
for one kept elsewhere; the in-app **scenario** picker lists them and switches without a
restart. An unknown name prints the available ones rather than failing obscurely.

The first build compiles the Bevy engine and takes several minutes; iterative rebuilds
are seconds (a fast-linker and dependency-optimisation profile are already configured).
Remove the `dynamic_linking` feature from `crates/app/Cargo.toml` before a release build.

Headless experiments run without a window:

```
cargo run -p experiments --bin pd_sweep        # detection model vs its closed form
cargo run -p experiments --bin duel_probe      # a direct-fire duel
cargo run -p experiments --bin sensor_siting   # sensor placement value
cargo run -p experiments --bin risk_path       # least-risk pathing
cargo run -p experiments --bin interdiction    # fires against a moving force
cargo run -p experiments --release --bin air_raid # drone raid vs air defence
cargo run -p experiments --release --bin allocation_gap  # coordinated vs uncoordinated fire
cargo run -p experiments --release --bin bench # hot-path timings
cargo run -p experiments --release --bin fires_bench     # the fires path alone
cargo run -p experiments --release --bin batch -- scenarios --seeds 50   # batch a folder
```

`batch` runs every scenario in a folder for N seeds and writes `out/<scenario>.csv` (a row
per seed) plus `out/summary.csv` (mean and standard error per scenario) — losses each
side, detections, time to first contact, and for air: launched, downed, leakers, munitions
released. The standard error sits beside every mean, because that is what says whether a
difference between two scenarios means anything.

## Subsystems

- **Terrain** — an elevation raster plus a terrain-type layer (open / trees / urban),
  with derived cover, concealment, LOS-blocking, and mobility-cost layers. Maps are
  generated from a seed, either as rolling relief or from a composable recipe (a base
  surface plus ordered ridge / woodland / urban layers).
- **Line of sight** — DDA traversal returning a rich result: blocked or clear, canopy
  transmittance, the height needed to clear the worst mask, and where the block occurred.
  Viewsheds rasterise the same query over the grid.
- **Sensing & detection** — glimpse-rate detection over LOS, slant range, target
  signature, and terrain concealment. Detection is mutual and asymmetric, so positioning
  to see without being seen is a real tactical decision. Tracks decay when observation
  lapses.
- **Fires** — direct fire gated on LOS and range with an error-function hit model;
  indirect fire as a ballistic arc to an aim point with CEP dispersion and Carleton area
  damage, with terrain shielding impacts.
- **Suppression & attrition** — units are N sub-elements with a discrete Free /
  Suppressed / Pinned Markov state driven by near misses, gating movement and fire.
- **Movement** — least-risk pathing over the terrain grid, trading exposure against time.
- **Air & counter-air** — drones as a third asset class: per-instance altitude above
  ground or sea level, turn-rate-limited flight, path or transit-then-orbit plans, and
  recce or strike payloads. Air defence answers with gun or missile engagement, gated by
  an envelope and by the sensor-to-shooter timeline.
- **Electronic warfare** — jamming as a modifier on the sensing channel, with a Bayesian
  belief filter that folds in spatial negative information.

## Validation

Every model ships with a test checking it against a closed-form result or a documented
invariant. The gates are numbered **V1–V59**, live in the `validation` crate, and each is
stated in `docs/DESIGN.md` next to the model it constrains, for example:

- **V14/V15** — the exponential detection law, Monte Carlo against `1 − e^(−λT)`
- **V22** — expected area damage against the Carleton–Gaussian convolution
- **V28** — the stationary distribution of the suppression Markov chain
- **V30** — attrition against Lanchester's square law
- **V40** — EW degrades detection, and EW switched off is exactly the identity
- **V48/V49** — air-defence time-to-kill: exponential for a gun, geometric for a missile
- **V55** — a track lapses without observation, so jamming can break one
- **V56** — the optimal allocation matches an exhaustive optimum, and beats no coordination
- **V57** — belief-driven tasking finds what a fixed stare never does
- **V59** — a C2 post makes air defence split a raid; killing it decoheres the defence

`cargo run -p validation --bin validation_report` prints every gate beside the closed form
it is checked against. That is the question the project actually cares about — not "are
the tests green" but *is the maths still right, and right against what*. A catalogue test
keeps the table from drifting out of step with the suite in either direction.

Rasterised layers (viewshed, coverage, belief, risk) are computed in parallel and remain
deterministic: no result depends on thread scheduling.

## Status

Roadmap phases 1–9 are implemented — terrain and LOS, sensing, fires, suppression and
attrition, movement as DP, game-theoretic decisions, visualisation, electronic warfare
with partial observability, and air with counter-air.

**Phase 10, the decision layer, is complete.** It closes the loop the phases above left
open — the simulation modelled everything except anyone *deciding* anything. Tracks now
decay (so jamming can break one, which permanent detection made impossible); fire is
allocated side-wide by solving an assignment problem; and steerable sensors point
themselves by expected information gain. Movement decisions in-loop are deliberately
deferred (DESIGN §10.5).

**Phase 11 adds command and control.** Coordination is modelled as an *asset you field*,
not a switch you set: a C2 post lets nearby air-defence batteries allocate as a group, and
destroying it costs no battery but decoheres the defence. Measured over 500 seeds, the
interesting effect is on **ammunition** rather than kills — a coordinated defence ends a
raid with four and a half times the magazine reserve, because stacking discrete missile
shots on one target is what actually wastes them.

Beyond that: suppression of enemy air defence, air-to-air, acoustic detection of drones,
ingesting real-world elevation data, live playback and state scrubbing, full-resolution
interactive path-planning and belief (currently coarse for interactivity), and a dynamic
stochastic game using DP value functions as payoffs.

## A note on scope

This models force-on-force dynamics at an abstract, doctrinal level — detection
probabilities, suppression states, attrition rates — using invented parameters chosen to
exercise the mathematics. It is a study of operational-research methods, not a source of
real-world capability data, and it is not calibrated against any real system.
