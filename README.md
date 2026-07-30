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
  decision-making are separate modules with clean interfaces — which is why electronic
  warfare could slot in as a modifier on the sensing channel rather than a rewrite.

## OR strands → subsystems

| Strand | Where it lives |
|---|---|
| **Optimal control** | Projectile ballistics and unit movement dynamics between decision epochs |
| **Dynamic programming** | Movement as least-risk pathing over the terrain grid (risk-weighted Dijkstra) |
| **Stochastic processes** | Probability of detection; hit/miss and impact dispersion (CEP); suppression as a Markov state; stochastic attrition |
| **Game theory** | Blue vs Red fires allocation and sensing vs counter-sensing, solved as a zero-sum game via fictitious play |
| **Partial observability** | A POMDP belief filter over enemy positions once electronic warfare degrades sensing |

The simulation loop is hybrid continuous/discrete: continuous state is integrated
between decision epochs, and discrete decisions (fire allocation, movement, sensor
tasking) are set at each epoch. That split is the optimal-control-plus-DP structure made
concrete.

## Layout

| Path | What it is |
|---|---|
| `crates/sim_core/` | Headless, deterministic OR engine — pure Rust, no Bevy. Where all the maths lives |
| `crates/app/` | Bevy front-end: tactical map, pan/zoom, egui control panel |
| `crates/experiments/` | Headless batch runs: sweeps, Monte Carlo, equilibria. Depends on `sim_core` only |
| `crates/validation/` | The V1–V52 gates: every model checked against a closed form or a stated invariant |
| `scenarios/` | TOML scenarios and the unit/weapon/sensor stat blocks |
| `docs/DESIGN.md` | The deep spec: equations, state machines, and the validation gate for every model |
| `SETUP.md` | Environment setup (Rust + Bevy + VSCode), written for a Rust beginner |

`sim_core` never depends on `app` or on Bevy. That boundary is what keeps the maths
independently testable and the simulation runnable headless.

## Quick start

```
cargo run -p app              # open the tactical map window
cargo test --workspace        # run the engine tests and the validation gates
cargo run -p validation --release --bin validation_report   # the V-gate table
cargo clippy --workspace      # lint
```

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
cargo run -p experiments --release --bin bench # hot-path timings
```

## Subsystems

- **Terrain** — an elevation raster plus a terrain-type layer (open / trees / urban),
  with derived cover, concealment, LOS-blocking, and mobility-cost layers. Terrain is
  generated procedurally from a seed.
- **Line of sight** — DDA traversal returning a rich result (blocked or clear, plus the
  accumulated concealment along the ray), with viewshed rasterisation over the grid.
- **Sensing & detection** — glimpse-rate detection: probability of detection as a
  stochastic function of LOS, range, target signature, and terrain-dependent
  concealment. Detection is mutual and asymmetric, so positioning to see without being
  seen is a real tactical decision.
- **Fires** — direct fire gated on LOS and range with an error-function hit model;
  indirect fire as a ballistic arc to an aim point with CEP dispersion and Carleton
  area damage, with terrain shielding impacts.
- **Suppression & attrition** — units are N sub-elements with a discrete Free /
  Suppressed / Pinned Markov state driven by near misses, gating movement and fire.
  Attrition is validated against Lanchester's square law.
- **Movement** — least-risk pathing over the terrain grid, trading exposure against time.
- **Game theory** — a zero-sum solver (fictitious play) over fires and sensing postures.
- **Electronic warfare** — jamming as a modifier on the sensing channel, with a Bayesian
  belief filter that also folds in spatial negative information ("I looked there and saw
  nothing").

## Validation

Every model ships with a test checking it against a closed-form result or a documented
invariant. The gates are numbered **V1–V52**, live in the `validation` crate, and each is
stated in `docs/DESIGN.md` next to the model it constrains, for example:

- **V14/V15** — the exponential detection law, Monte Carlo against `1 − e^(−λT)`
- **V28** — the stationary distribution of the suppression Markov chain
- **V30** — attrition against Lanchester's square law
- **V40** — EW degrades detection, and EW switched off is exactly the identity
- **V48/V49** — air-defence time-to-kill: exponential for a gun, geometric for a missile

`cargo run -p validation --bin validation_report` prints every gate beside the closed form
it is checked against, which is the question the project actually cares about — not "are
the tests green" but *is the maths still right, and right against what*. A catalogue test
keeps that table from drifting out of step with the suite in either direction.

Rasterised layers (viewshed, coverage, belief, risk) are computed in parallel and remain
deterministic: no result depends on thread scheduling.

## Status

All eight roadmap phases are implemented — terrain and LOS, sensing, fires, suppression
and attrition, movement as DP, game-theoretic decisions, visualisation, and electronic
warfare with partial observability — plus a ninth on air: drones as a third asset class
(altitude above ground or sea level, turn-rate-limited flight, path or transit-then-orbit
plans, reconnaissance and strike payloads) answered by air defence (gun or missile, each
with its own time-to-kill law, gated by an engagement envelope and by the sensor-to-shooter
timeline).

Possible next steps: kill-chain modelling (autonomous target selection and sensor-to-shooter
pairing), suppression of enemy air defence, ingesting real-world elevation data, live
playback and state scrubbing, full-resolution interactive path-planning and belief
(currently coarse for interactivity), further sensor modalities such as acoustic and
EO/IR, and a dynamic stochastic game using DP value functions as payoffs.

## A note on scope

This models force-on-force dynamics at an abstract, doctrinal level — detection
probabilities, suppression states, attrition rates — using invented parameters chosen to
exercise the mathematics. It is a study of operational-research methods, not a source of
real-world capability data, and it is not calibrated against any real system.
