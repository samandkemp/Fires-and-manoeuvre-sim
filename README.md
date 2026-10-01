# Fires & Manoeuvre Sim

An operational-research simulation of land warfare direct and indirect fires. A Blue and a Red
force – composed from artillery, manoeuvre units, sensors, drones and air defence – fight over
featured terrain: line of sight, cover, concealment, mobility.

**Sensing is central.** You place sensors and try to detect the enemy before being detected,
then watch fires suppress and attrit manoeuvre. Detection is mutual and asymmetric, so
positioning to see without being seen is a real decision rather than a scoring bonus.

Written in Rust, with a Bevy front-end for the tactical map and a headless core for the maths.

> **All unit, weapon and sensor numbers are abstract placeholder dials, not real munition or
> sensor performance data.** The models are the product; the numbers are dials to be turned.

## What it is for

A personal research and learning tool, built to make operational-research models tangible and
tweakable rather than to ship as a game. The goal is to take six bodies of theory that are
usually taught separately – optimal control, dynamic programming, stochastic processes, game
theory, partial observability, combinatorial optimisation – and make each one *do something
visible* in the same simulation, where its consequences can be measured against the others.

Three commitments follow from that, and they override convenience:

- **The maths is the product.** Every model is formulated and validated against a known
  analytical result or a documented invariant *before* it is made fast or pretty. Correctness is
  testable; "realism" is not. There are 77 such gates.
- **Headless and deterministic.** The simulation is a pure library given a scenario and a seed,
  producing identical results with no UI attached. As a result, ten thousand trials need no
  window; they run in under twenty seconds, byte-identical to a serial run.
- **Data-driven.** Unit, weapon and sensor stats live in TOML, never hard-coded, so they are
  tweakable at runtime and sweepable by dotted path without editing a file.

One structural discipline underpins all of it: **every subsystem added reduces to an exact
identity when switched off.** A scenario with no aircraft produces the event log it did before
the air model existed, byte for byte rather than approximately. Every phase of additions has
been made safe that way.

## Who it is for

Written for someone comfortable with code and curious about how these methods behave when they
have to share a simulation, whether or not they have met operational research before. The
documentation explains the vocabulary rather than assuming it, and the guide is written so that
somebody new to the model could set it up, build a scenario and run a defensible study without
reading any of the theory.

It is a study of operational-research *methods*, not a source of real-world capability data, and
it is not calibrated against any real system.

## Six strands of theory

Each does a job the others cannot.

| Strand | Doing what | Lives in |
|---|---|---|
| **Optimal control** | Turn-rate-limited flight; phase-integrated orbits | `airframes.rs` |
| **Dynamic programming** | Least-risk pathing: Dijkstra as label-setting value iteration | `movement.rs` |
| **Stochastic processes** | Detection rates, CEP (Circular Error Probable) dispersion, the suppression chain, time-to-kill | `sensing.rs`, `weapon_effects.rs`, `suppression.rs`, `air_defence.rs` |
| **Game theory** | Sensing against counter-sensing, by fictitious play | `game_theory.rs` |
| **Partial observability** | Belief over enemy position, and the value of *not* seeing | `ew.rs`, `pomdp.rs` |
| **Combinatorial optimisation** | Side-wide weapon-target assignment (Kuhn-Munkres) | `allocation.rs` |

The loop is **hybrid continuous/discrete**: between decision epochs the state integrates
continuously; at each epoch the discrete decisions are set: what to shoot, where to move, where
to look. That split is not a convenience; it is the structure. Continuous dynamics are an
optimal-control problem, the epoch-to-epoch choices are a dynamic program, and the two only
compose cleanly if they are kept apart.

## Documentation

Three layers. Start at the top and go down only as far as your question needs.

| | Document | For |
|---|---|---|
| **1** | this page | What it is, and where everything is |
| **2** | **[docs/GUIDE.md](docs/GUIDE.md)** | **Setting up, running, authoring scenarios, running studies.** The single source for operating the model; start here to *use* it |
| **2** | [docs/MODEL.md](docs/MODEL.md) | How it works and how the parts interact: the tick, detection, engagement, with worked numbers and the code path for each. Start here to *understand* it |
| **2** | [docs/REFERENCE.md](docs/REFERENCE.md) | Every TOML field, `[sim]` dial, metric and command-line flag. The lookup table |
| **3** | [docs/THEORY.md](docs/THEORY.md) | The specification: every model derived from its general form, what was rejected on the way, and its limitations |
| **3** | [docs/VALIDATION.md](docs/VALIDATION.md) | The 77 gates, what each is checked *against*, and how to add one |

### Two numbering schemes

The prose leans on both, so they are worth thirty seconds up front:

- **§N.M** is a section of [docs/THEORY.md](docs/THEORY.md); §10.2 is fire allocation. It is
  referenced from over four hundred places in the source, which is why sections are never
  renumbered.
- **V1-V77** is a *validation gate*: one property checked against a closed form or a documented
  invariant. V25 is "zero risk weight gives the shortest path".

Neither is a hierarchy to be learned. They are stable names, so a claim made in one place can be
checked in another.

### Abbreviations

The documentation expands each of these on first use, but they are gathered here so that a reader
arriving part-way through has somewhere to look.

| | | | |
|---|---|---|---|
| **AD** | Air Defence | **EMCON** | Emission Control |
| **AGL** | Above Ground Level | **EW** | Electronic Warfare |
| **AMSL** | Above Mean Sea Level | **LOS** | Line of Sight |
| **ARM** | Anti-Radiation Missile | **OR** | Operational Research |
| **C2** | Command and Control | **POMDP** | Partially Observable Markov Decision Process |
| **CEP** | Circular Error Probable | **SAM** | Surface-to-Air Missile |
| **CIWS** | Close-In Weapon System | **SEAD** | Suppression of Enemy Air Defences |
| **DEM** | Digital Elevation Model | **TTK** | Time to Kill |
| **DP** | Dynamic Programming | **UAS** | Uncrewed Aerial System |

## Layout

```
crates/sim_core/     the OR engine - pure Rust, no Bevy. Where all the maths lives
crates/app/          Bevy front-end: tactical map, pan/zoom, egui control panel
crates/experiments/  headless studies: batch, sweep, factorial, sensitivity
crates/validation/   the V1-V77 gates, checked through the public API only
scenarios/           TOML scenarios and the unit/weapon/sensor stat blocks
studies/             dial-space designs for global sensitivity analysis
docs/                the five documents above
```

The dependency arrows only point one way. **`sim_core` never depends on `app` or on Bevy.** That
boundary is what keeps the maths independently testable and the simulation runnable headless, and
it is the one rule in the project that is never bent.

## Quick start

```
cargo run -p app                       # open the tactical map (the `default` scenario)
cargo run -p app -- air_raid           # open a named scenario from scenarios/
cargo test --workspace                 # the engine tests and the validation gates
cargo run -p validation --release --bin validation_report   # the gate table
```

The first build compiles the Bevy engine and takes several minutes; iterative rebuilds are
seconds.

Then ask it something:

```
# Does coordinating fires matter, and by how much?
cargo run -p experiments --release --bin sweep -- fire_allocation \
    --param sim.allocation --values independent,greedy,optimal \
    --seeds 500 --metric red_cleared_s
```

```
--- red_cleared_s, paired against sim.allocation = independent ---
  sim.allocation = independent  baseline 75.355
  sim.allocation = greedy      -12.835 +- 0.224 (t = -57.2, n = 2000, 311 tied) significant
  sim.allocation = optimal     -12.430 +- 0.231 (t = -53.8, n = 2000, 323 tied) significant
```

Coordinating clears the enemy about 12.8 s sooner, comfortably outside the noise. However,
solving the assignment *optimally* rather than greedily is **worse**, by 0.405 ± 0.051 s when the
two are compared against each other directly (t = 8.0). This is not a defect in the solver: the
allocation objective scores a single epoch, so solving it exactly is myopically right and can cost
more over a whole engagement than a greedy rule that happens to spread fire. That is the useful
kind of negative result, and it exists only because the greedy baseline was kept rather than
deleted once the optimal solver worked.

See [docs/GUIDE.md](docs/GUIDE.md) for everything else.

## Status

The model covers terrain and line of sight, sensing and detection, direct and indirect fires,
suppression and attrition, movement as dynamic programming, a game-theoretic layer, electronic
warfare with partial observability, air and counter-air, a decision layer closing sensing to
action, command and control as a placed asset, SEAD (Suppression of Enemy Air Defences), directed
targeting, and movement decisions taken inside the loop. Alongside it sits a study harness for
batch runs, sweeps, factorial designs and global sensitivity analysis. All 77 validation gates
hold.

Each theory section states the limitations its model accepts. The largest open one is that the
fire-allocation objective scores a single epoch, which is measurably what costs the optimal solver
against a greedy rule
([§10.6](docs/THEORY.md#106-the-planning-horizon-and-what-the-optimal-versus-greedy-gap-is-made-of)).
