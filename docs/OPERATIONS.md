# Operations

Every command for building, running, inspecting and testing the model, in one place.
Environment setup — installing Rust, the linker, the VSCode extensions — is separate, in
[`SETUP.md`](../SETUP.md).

Related: [`docs/SCENARIOS.md`](SCENARIOS.md) for *writing* a scenario,
[`docs/EXPERIMENTS.md`](EXPERIMENTS.md) for designing a study,
[`docs/VALIDATION.md`](VALIDATION.md) for the gates.

## The short version

```
cargo run -p app                      # open the tactical map on the `default` scenario
cargo test --workspace                # everything: gates, harness, app
cargo clippy --workspace              # lint
cargo fmt                             # format
```

The first build compiles the Bevy engine and takes several minutes. Iterative rebuilds are
seconds — a fast linker and a dependency-optimisation profile are already configured.

## Running the app

```
cargo run -p app                       # the `default` scenario
cargo run -p app -- air_raid           # by bare name, resolved in scenarios/
cargo run -p app -- path/to/mine.toml  # or by path, for one kept elsewhere
```

An unknown name prints the scenarios it could have opened rather than failing obscurely.
The in-app **scenario** dropdown lists every file in `scenarios/` that parses as a scenario
and switches between them live — terrain, forces and all, no restart.

### Controls

The UI is **modeless**. Selection is a set, and the only mode is what a right-click places.

| Input | Does |
|---|---|
| Left-click | Select — units, drones, AD batteries or C2 posts |
| Shift + left-click | Add to / toggle in the selection |
| Left-drag | Box-select |
| Right-click | Move the selection here, preserving formation |
| Shift + right-click | Append a waypoint instead of replacing the route |
| Ctrl + A | Select all live assets |
| Del | Remove the selection |
| Esc | Clear the selection |
| Middle-drag / scroll | Pan / zoom |
| **Space** | Run / pause |
| **.** | Step one tick |

Space and `.` are on the keyboard as well as the panel deliberately: inspecting a battle
means keeping your eyes on the map, and reaching for a button loses the moment you paused
for.

### Watching a battle unfold

A scenario resolves in a couple of hundred sim-seconds, which is faster than it can be
watched. The clock panel exists for that:

| Control | What it does |
|---|---|
| **speed** | **Sim seconds per real second.** 0.2× to study a duel, 60× to skip ahead |
| **+1 s / +10 s** | Step one integration tick, or one decision epoch — the two units the model actually has |
| **pause on** *contact* / *loss* / *air* | Breakpoints: stop *on* the tick that produced the event |
| **Run to** | Jump ahead at headless speed to a given sim time |
| **Re-run at seed** | Replay the same battle exactly |
| **Reset scenario** / **Clear all** | Reload from file, or empty the map to build one by hand |

**Speed cannot change the outcome.** The wall clock decides *when* a tick happens, never
how big it is: real time accumulates into a budget which is spent in whole `dt_s` ticks. So
0.2× and 60× produce the same event log, and the slow one is a magnifying glass rather than
a different experiment.

Breakpoints matter more than they sound. The moments worth seeing — first contact, a
casualty, a missile away — last a single tick, so slowing down is not enough on its own;
you also have to be looking at the right pixel at the right moment. A breakpoint stops the
clock on the tick that tripped it.

### Overlays and inspection

| Button | Shows |
|---|---|
| **Coverage overlay (Pd)** | Detection probability across the map for the placed sensors |
| **Belief snapshot** | Where Red could be, given what Blue has and has not seen |
| **Belief the sim is flying on (Blue)** | The sim's own per-side belief — what tasking is actually reading |
| **Legend** | What every marker and colour means |

Live dials in the panel — fire allocation, sensor tasking, the air and decision-layer
settings — take effect immediately, so you can watch a rule change the battle rather than
re-reading a CSV.

### Screenshots

```
FIRES_SIM_SCREENSHOT=out/shot.png cargo run -p app -- air_raid
```

Captures one frame a few seconds in and exits. The rig runs on a frame-locked path so its
capture time is reproducible rather than dependent on how fast the machine drew.

## Testing

```
cargo test --workspace                 # all of it
cargo test -p validation               # the V1-V66 gates
cargo test -p sim_core                 # engine unit tests (fast, headless)
cargo test -p experiments              # harness: parallel-equals-serial, patching, statistics
cargo test -p app                      # selection and picking
```

```
cargo run -p validation --release --bin validation_report
```

That last one is the one worth running. It prints every gate beside **the closed form it is
checked against**, because the useful question is not "are the tests green" but *is the
maths still right, and right against what*. See [`docs/VALIDATION.md`](VALIDATION.md).

## Experiments

Two general tools sit on a shared harness: `batch` compares **scenarios**, `sweep` compares
**dials**. Everything else is a bespoke probe.

### `batch` — a folder of scenarios

```
batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]
```

```
cargo run -p experiments --release --bin batch -- scenarios --seeds 50
cargo run -p experiments --release --bin batch -- scenarios --only air_raid --seeds 10000
```

Writes `out/<scenario>.csv` (a row per seed) and `out/summary.csv` (mean and standard error
per scenario). `--seeds N` always means seeds `0..N`, so "run 1,000 more" is `--seeds 2000`.

### `sweep` — one dial, many values

```
sweep <scenario> --param PATH (--values a,b,c | --from X --to Y [--steps N])
                 [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...
                 [--dir DIR] [--out DIR] [--quiet]
```

```
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

`--param` is a **dotted path**, patched into the TOML before it is parsed, so any dial is
sweepable — including one added next month — and the patched file goes through exactly the
same validation as one on disk. It reaches the **stat-block libraries** as well as the
scenario, which is where the models actually live:

```
sweep air_raid --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 --seeds 1000
sweep default  --param weapons.mortar.cep_m --from 20 --to 140 --steps 7 --seeds 2000
sweep ad_c2    --param sim.max_batteries_per_air_target --values 1,2,3 \
               --set sim.allocation=greedy --seeds 1000
```

Every comparison is **paired**: all arms run the same seed set, so the map and most of the
luck cancel. There is no unpaired option, because an unpaired comparison once produced a
confident and entirely spurious finding about the allocation solvers.

Trials run in parallel, one sim per worker: **10,000 trials in under 20 seconds**, and
byte-identical to a serial run — pinned by a test, because "we parallelised it and the
answer changed" is otherwise discovered months later by a confusing result.

### The bespoke probes

Each answers one question and prints a table. They pre-date the general harness and are
kept because each one *demonstrates* a model rather than measuring a dial.

```
cargo run -p experiments --bin pd_sweep            # detection model vs its closed form
cargo run -p experiments --bin duel_probe          # a direct-fire duel
cargo run -p experiments --bin sensor_siting       # what a sensor position is worth
cargo run -p experiments --bin risk_path           # least-risk pathing vs the shortest route
cargo run -p experiments --bin interdiction        # fires against a moving force
cargo run -p experiments --release --bin air_raid          # a drone raid vs air defence
cargo run -p experiments --release --bin allocation_gap    # coordinated vs uncoordinated fire
```

### Benchmarks

```
cargo run -p experiments --release --bin bench        # LOS, viewshed, slant range, tick cost
cargo run -p experiments --release --bin fires_bench  # the fires path alone
```

`bench` also reports the LOS memo hit rate. Note that **terrain build time is the figure
that matters** — the sim tick is sub-millisecond and too noisy to optimise against. The
fires path gets its own bench because the tick bench is too sensing-dominated to resolve
it.

## Building for release

```
cargo build -p app --release
```

**Remove the `dynamic_linking` feature from `crates/app/Cargo.toml` first.** It makes
iterative builds much faster and produces a binary that will not run elsewhere.

## Lint and format

```
cargo clippy --workspace              # take its advice seriously
cargo clippy --workspace --all-targets   # including tests and benches
cargo fmt                             # rustfmt; format-on-save is configured
```

## A note on determinism

Everything above is reproducible from `(scenario, seed)`. Nothing reads the wall clock,
nothing depends on thread scheduling, and the parallel rasters and parallel trials are
checked to produce bit-identical results to their serial equivalents. If two runs of the
same command disagree, that is a bug, not variance.
