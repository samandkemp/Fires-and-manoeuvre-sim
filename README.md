# Fires & Manoeuvre Sim

An operational-research simulation of land warfare direct and indirect fires. A Blue and a
Red force — composed from artillery, manoeuvre units, sensors, drones and air defence —
fight over featured terrain: line of sight, cover, concealment, mobility.

**Sensing is central.** User places sensors and try to detect the enemy before being
detected, then watch fires suppress and attrit manoeuvre. Detection is mutual and
asymmetric, so positioning to see without being seen is a real decision rather than a
scoring bonus.

Written in Rust, with a Bevy front-end for the tactical map and a headless core for the
maths. It is a personal research and learning tool, built to make OR models tangible and
tweakable — not to ship as a game.

> **All unit, weapon and sensor numbers are abstract placeholder dials — not real munition
> or sensor performance data.** The models are the product; the numbers are knobs.

## What it is built around

- **Headless, deterministic core.** The simulation is a pure library (`sim_core`, no
  Bevy): given a scenario and a seed it produces identical results with no UI attached, so
  ten thousand trials need no window. 10,000 trials run in under 20 seconds, byte-identical
  to a serial run.
- **The maths is the product.** Every model is formulated and validated against a known
  analytical result or a documented invariant *before* it is made fast or pretty.
  Correctness is testable; "realism" is not. There are 69 such gates.
- **Data-driven.** Unit, weapon and sensor stats live in TOML, never hard-coded, so they
  are tweakable at runtime — and sweepable by dotted path without editing a file.
- **Composable subsystems.** Terrain, fires, sensing, suppression, movement and
  decision-making are separate modules with clean interfaces, which is why electronic
  warfare slotted in as a modifier on the sensing channel rather than a rewrite.

One structural discipline underpins all of it: **every subsystem added reduces to an exact
identity when switched off.** A scenario with no aircraft produces the event log it did
before the air model existed — byte for byte, not approximately. Eight phases of additions
have been made safe that way.

## Six strands of theory

Each does a job the others cannot. [`docs/MATHS.md`](docs/MATHS.md) states each in its own
symbols, with the code it lives in and the gate that holds it honest.

| Strand | Doing what | Lives in |
|---|---|---|
| **Optimal control** | Turn-rate-limited flight; phase-integrated orbits | `air.rs` |
| **Dynamic programming** | Least-risk pathing — Dijkstra as label-setting value iteration | `movement.rs` |
| **Stochastic processes** | Detection rates, CEP dispersion, the suppression chain, time-to-kill | `sensing.rs`, `fires.rs`, `suppression.rs`, `air_defence.rs` |
| **Game theory** | Sensing against counter-sensing, by fictitious play | `game.rs` |
| **Partial observability** | Belief over enemy position, and the value of *not* seeing | `ew.rs`, `pomdp.rs` |
| **Combinatorial optimisation** | Side-wide weapon–target assignment (Kuhn–Munkres) | `allocation.rs` |

The loop is **hybrid continuous/discrete**: between decision epochs the state integrates
continuously; at each epoch the discrete decisions are set — what to shoot, where to move,
where to look. That split is not a convenience, it is the structure. Continuous dynamics
are an optimal-control problem, the epoch-to-epoch choices are a dynamic program, and the
two only compose cleanly if they are kept apart.

## Subsystems

**Terrain** — an elevation raster plus a terrain-type layer, with derived cover,
concealment, LOS-blocking and mobility layers. Maps are generated from a seed, either as
rolling relief or from a composable recipe (a base surface plus ordered ridge / woodland /
urban layers). **Line of sight** — DDA traversal returning canopy transmittance, the height
needed to clear the worst mask, and where the block occurred.

**Sensing** — glimpse-rate detection over LOS, slant range, signature and concealment.
Tracks decay when observation lapses, which is what lets jamming *break* a track rather
than only prevent one. **Fires** — direct fire gated on LOS with an error-function hit
model; indirect fire as a ballistic arc with CEP dispersion and Carleton area damage.
**Suppression** — units are N sub-elements with a Free / Suppressed / Pinned Markov state
driven by near misses, gating movement and fire.

**Air** — drones as a third asset class: per-instance altitude above ground or sea level,
turn-rate-limited flight, path or transit-then-orbit plans, recce or strike payloads. **Air
defence** answers with gun or missile engagement, gated by an envelope and by the
sensor-to-shooter timeline. **Command and control** is an asset you field, not a switch you
set: a post lets nearby batteries allocate as a group, and it can be jammed or killed.

**The decision layer** closes the loop sensing → belief → decision → action. Fire is
allocated side-wide by solving an assignment problem; steerable sensors point themselves by
expected information gain; and a **kill chain** lets a side declare what it has been *told*
to shoot first, so directive control can be measured against optimal control.

## Documentation

| Doc | Read it for |
|---|---|
| **[docs/HOW_IT_WORKS.md](docs/HOW_IT_WORKS.md)** | **Start here.** The model in plain terms: the modules, one tick, how detection and engagement actually work, with worked numbers |
| [docs/MATHS.md](docs/MATHS.md) | The six OR strands stated properly, and the argument for each |
| [docs/design/](docs/design/) | The specification: equations, state machines and invariants, one page per section |
| [docs/VALIDATION.md](docs/VALIDATION.md) | The V1–V69 gates, what each is checked *against*, and how to add one |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Every command: build, run, playback controls, tests, experiments, benches |
| [docs/SCENARIOS.md](docs/SCENARIOS.md) | Adding a unit / weapon / sensor / drone / battery, and building a scenario |
| [docs/EXPERIMENTS.md](docs/EXPERIMENTS.md) | Designing a study: batch runs, sweeps, paired statistics, reading the output |
| [docs/ROADMAP.md](docs/ROADMAP.md) | What comes next, in what order, and the decisions already taken |
| [SETUP.md](SETUP.md) | Environment setup, written for a Rust beginner |

## Layout

```
crates/sim_core/     the OR engine — pure Rust, no Bevy. Where all the maths lives
crates/app/          Bevy front-end: tactical map, pan/zoom, egui control panel
crates/experiments/  headless batch runs: sweeps, Monte Carlo, equilibria
crates/validation/   the V1–V69 gates, checked through the public API only
scenarios/           TOML scenarios and the unit/weapon/sensor stat blocks
docs/                see the table above
```

The dependency arrows only point one way. **`sim_core` never depends on `app` or on Bevy** —
that boundary is what keeps the maths independently testable and the simulation runnable
headless, and it is the one rule in the project that is never bent.

Inside `sim_core`, `sim/` is the engine that drives everything else. The model code around
it — `sensing.rs`, `fires.rs`, `movement.rs` — is pure functions with no state, which is
what lets the validation crate check each one in isolation.

## Quick start

```
cargo run -p app                       # open the tactical map (the `default` scenario)
cargo run -p app -- air_raid           # open a named scenario from scenarios/
cargo test --workspace                 # the engine tests and the validation gates
cargo run -p validation --release --bin validation_report   # the gate table
```

```
# Does coordinating fires matter, and by how much?
cargo run -p experiments --release --bin sweep -- fire_allocation \
    --param sim.allocation --values independent,greedy,optimal \
    --seeds 500 --metric red_cleared_s
```

```
--- red_cleared_s, paired against sim.allocation = independent ---
  sim.allocation = independent  baseline 75.080
  sim.allocation = greedy      -11.300 +- 0.473 (t = -23.9, n = 500, 93 tied) significant
  sim.allocation = optimal     -11.180 +- 0.487 (t = -23.0, n = 500, 90 tied) significant
```

Coordinating clears the enemy 11.3 s sooner, far outside the noise. Solving the assignment
*optimally* rather than greedily is worth 0.12 s — a quarter of one standard error, i.e.
nothing. That is the useful kind of negative result, and it only exists because the greedy
baseline was kept rather than deleted once the optimal solver worked.

The first build compiles the Bevy engine and takes several minutes; iterative rebuilds are
seconds. See [docs/OPERATIONS.md](docs/OPERATIONS.md) for everything else.

## Status

Roadmap phases 1–14 are complete: terrain and LOS, sensing, fires, suppression and
attrition, movement as DP, game-theoretic decisions, visualisation, electronic warfare with
partial observability, air and counter-air, the decision layer, command and control, SEAD,
and the kill chain. V1–V69 all hold.

Next: the allocation surrogate (a multi-epoch objective), air-to-air, acoustic detection of
drones, real-world DEM ingestion, movement decisions in-loop, and a dynamic stochastic game
using DP value functions as payoffs.

## A note on scope

This models force-on-force dynamics at an abstract, doctrinal level — detection
probabilities, suppression states, attrition rates — using invented parameters chosen to
exercise the mathematics. It is a study of operational-research methods, not a source of
real-world capability data, and it is not calibrated against any real system.
