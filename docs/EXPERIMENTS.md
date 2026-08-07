# Running experiments

How to get a number out of this simulation that you can defend.

The app shows one battle. That is the wrong tool for a question, because one battle is one
draw from a distribution and it is not the interesting one. This document is about the
other half: running the same situation thousands of times and reporting what actually
changed, with an error bar.

This is the **study-design** document — why paired, what the columns mean, how to read the
output, and what the harness cannot yet do. For the bare command syntax see
[`docs/OPERATIONS.md`](OPERATIONS.md); for the dials themselves see
[`docs/SCENARIOS.md`](SCENARIOS.md).

- [The five-minute version](#the-five-minute-version)
- [The two rules](#the-two-rules)
- [`batch` — a folder of scenarios](#batch--a-folder-of-scenarios)
- [`sweep` — one dial, many values](#sweep--one-dial-many-values)
- [What the columns mean](#what-the-columns-mean)
- [Reading the output](#reading-the-output)
- [How fast, and why](#how-fast-and-why)
- [Worked example: is the overkill cap earning its keep?](#worked-example-is-the-overkill-cap-earning-its-keep)
- [Adding a metric](#adding-a-metric)
- [Adding an experiment](#adding-an-experiment)
- [The bespoke binaries](#the-bespoke-binaries)
- [What this harness cannot do yet](#what-this-harness-cannot-do-yet)

---

## The five-minute version

```bash
# Every scenario, 50 seeds each, CSV into out/
cargo run -p experiments --release --bin batch -- scenarios --seeds 50

# One scenario, 10,000 seeds
cargo run -p experiments --release --bin batch -- scenarios --only air_raid --seeds 10000

# Does the fire-allocation solver matter, and by how much?
cargo run -p experiments --release --bin sweep -- fire_allocation \
    --param sim.allocation --values independent,greedy,optimal \
    --seeds 500 --metric red_cleared_s
```

That last command prints:

```
--- red_cleared_s, paired against sim.allocation = independent ---
  sim.allocation = independent  baseline 75.080
  sim.allocation = greedy      -11.300 +- 0.473 (t = -23.9, n = 500, 93 tied) significant
  sim.allocation = optimal     -11.180 +- 0.487 (t = -23.0, n = 500, 90 tied) significant
```

Read it as: coordinating fires clears the enemy 11.3 s sooner, and that is far outside the
noise. Solving the assignment *optimally* rather than greedily is worth 0.12 s, which is a
quarter of one standard error — nothing. Both arms gave literally the same answer on about
90 of the 500 seeds.

Always `--release`. A debug build of a Monte Carlo study is roughly 20× slower and tells
you nothing a release build does not.

---

## The two rules

Both exist because breaking them produces a confident wrong answer rather than an obvious
failure.

### 1. Fix the map, vary the dice

`Sim::new(scenario, libs, seed)` derives **both** the terrain and the RNG stream from that
one seed. Looping it over seeds therefore varies the map and the luck together, and the
result averages two sources of variance that answer different questions.

Every study here builds the terrain **once per worker, from the scenario's own
`default_seed`**, and calls `Sim::reset_to_scenario` between trials. So the map is held
fixed and the question is "what happens on *this* map, on average". It is also far faster:
terrain generation is seconds, a trial is microseconds.

If you *want* to average over maps — a fair question, just a different one — sweep
`default_seed` itself:

```bash
sweep default --param default_seed --values 1,2,3,4,5 --seeds 400
```

### 2. Compare paired, always

Two arms of a study run the **same seed set**, so arm A and arm B are matched trial for
trial: same map, same dice, one dial different. The difference is then taken seed by seed:

```
  d_k = a_k − b_k          SE(d̄) = s_d / √n
```

This is **common random numbers**, and it works because

```
  Var(a − b) = Var(a) + Var(b) − 2·Cov(a, b)
```

The two arms share the map and most of the luck, so `Cov(a, b)` is large and positive and
most of the variance cancels. Comparing the two *unpaired* means throws that away and can
be an order of magnitude noisier.

That is not hypothetical. Comparing the allocation solvers unpaired once produced a
confident finding that greedy **beat** the optimal assignment. Paired over 500 seeds the
difference was 0.12 s against an SE of 0.5, and the two were *identical* on 438 of the 500
seeds (`DESIGN.md` §10.2). The whole "effect" was the variance that pairing cancels.

`experiments::stats` therefore offers `paired()` and no unpaired comparison at all, and
`paired()` **panics** if the two arms are different lengths rather than quietly comparing
what it has.

---

## `batch` — a folder of scenarios

```
batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]
```

| Flag | Default | What it does |
|---|---|---|
| `dir` | `scenarios` | folder of scenarios **and** stat-block libraries |
| `--seeds` | 20 | trials per scenario; seeds `0..N` |
| `--until` | 600 | sim seconds per trial |
| `--out` | `out` | where the CSVs go |
| `--only` | — | just this scenario, by bare name |
| `--quiet` | — | no progress line |

Writes `out/<scenario>.csv` (a row per seed, every metric) and `out/summary.csv` (a row per
scenario, mean and SE for every metric). `.gitignore` already covers `out/` and `*.csv`.

`batch` compares **scenarios**. It is the regression sweep — run it after a change to see
whether anything moved that should not have. To compare **dials**, use `sweep`.

## `sweep` — one dial, many values

```
sweep <scenario> --param PATH (--values a,b,c | --from X --to Y [--steps N])
                 [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...
                 [--dir DIR] [--out DIR] [--quiet]
```

`--param` is a **dotted path**, into either the scenario or a stat-block library:

```bash
sweep air_raid  --param sim.track_hold_s   --values 10,20,45,90        --seeds 500
sweep default   --param sim.p_suppress     --from 0.05 --to 0.8 --steps 8 --seeds 2000
sweep air_raid  --param red.air.0.altitude_m --values 150,300,600,1200 --seeds 1000
sweep ad_c2     --param sim.allocation     --values greedy,optimal     --seeds 1000 \
                --set sim.max_batteries_per_air_target=1

# stat blocks: the sensor, weapon and terrain dials the OR models actually turn on
sweep air_raid  --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 --seeds 1000
sweep default   --param weapons.mortar.cep_m       --from 20 --to 140 --steps 7 --seeds 2000
sweep default   --param terrain_types.trees.concealment --values 0.2,0.5,0.8 --seeds 1000
```

The **first segment decides which file** is patched: `sensors`, `units`, `weapons`, `air`,
`air_defence`, `c2` and `terrain_types` name library files, and anything else is a scenario
path. The two namespaces cannot collide, because a scenario's top level is `name`,
`default_seed`, `terrain`, `sim`, `blue` and `red`. So `air.recce.speed_m_s` is the *stat
block* and `red.air.0.speed_m_s` is one airframe's override of it.

Note that a `terrain_types` sweep changes the **map**, not just what happens on it, so each
arm rebuilds terrain. That is a legitimate question, just a slower and noisier one.

The override is applied to the **TOML**, before it is parsed. Three consequences:

- Any field is sweepable, including dials added after this document was written. `sweep`
  has no knowledge of the scenario schema.
- Numeric path segments index arrays (`red.air.0.altitude_m`), and string-valued dials work
  as-is (`sim.allocation=greedy`).
- The patched scenario goes through **exactly the same loader** as a file on disk, so an
  out-of-range value fails the same way rather than reaching the sim.

A dial that is absent from the file — most of them, since nearly all have a default — is
created. That is safe because the schema sets `deny_unknown_fields` throughout, scenario
and stat blocks alike: `sim.track_hold` for `sim.track_hold_s` is a load error naming the
key, not a silent default. (Which also means a typo in a hand-written scenario or stat
block is now caught, instead of quietly changing what it means.)

`--set` is repeatable and applies to **every** arm, for holding one dial away from the
scenario's own setting while varying another.

`--metric` chooses which column the paired report is about; every column is still in the
CSV. Output goes to `out/<scenario>_<param>.csv` and `..._summary.csv`.

---

## What the columns mean

Every metric is read back from the sim's own event logs and final state — never accumulated
alongside the sim as it runs. So there is no second bookkeeping path to drift: if a metric
is wrong, the log is wrong, and the app's event feed is showing the same wrong thing.

| Column | Meaning |
|---|---|
| `blue_losses`, `red_losses` | ground sub-elements destroyed |
| `blue_units_killed`, `red_units_killed` | whole units reduced to zero elements |
| `detections` | detection events, both sides, ground + air |
| `first_detection_s` | time of first contact either way; the run length if none |
| `fire_events` | fires resolutions that produced casualties |
| `air_launched`, `air_downed` | airframes placed, and shot down |
| `air_leakers` | airframes that survived to **release** a munition |
| `munitions_released` | munitions released |
| `ground_casualties_from_air` | elements killed by air-delivered bursts |
| `ad_shots` | air-defence shots taken — the denominator for rounds-per-kill |
| `ad_rounds_left` | interceptors remaining, finite magazines only |
| `ad_batteries_killed`, `c2_posts_killed` | what SEAD is trying to achieve |
| `blue_cleared_s`, `red_cleared_s` | when a side lost its last ground element |
| `epochs` | decision epochs resolved |

Two that repay attention:

**`*_cleared_s` is usually the metric that answers "was this better?"** Losses saturate: if
everything on one side dies by 600 s in every arm, `red_losses` is the same number
everywhere and only the *time* distinguishes them. That is how the Phase 10 allocation
result had to be measured. A side that was never cleared reports the run length, so read it
with the kill counts beside it — `600` means "not by 600 s", not "at 600 s". A side that
fields no ground units at all also reports the run length; there was nothing to clear.

**`ad_rounds_left` is where the Phase 11 C2 result lives.** Coordinating air defence barely
changes how many drones die; it changes how much ammunition is left afterwards, because a
missile is a discrete round and overkill is real. A gun is a Poisson process, so stacking
guns simply adds kill rates and wastes nothing. Coordination pays where the shot is a
countable resource — and that is invisible without this column.

---

## Reading the output

```
sim.allocation = greedy      -11.300 +- 0.473 (t = -23.9, n = 500, 93 tied) significant
```

- **`-11.300`** — the mean paired difference against the *first* arm, in the metric's units.
- **`+- 0.473`** — its standard error. Roughly: the true value is within about 2 of these.
- **`t = -23.9`** — `mean / SE`. `|t| > 2` is the line this harness calls significant
  (two-sided, ~5%). At 23.9 there is nothing to argue about.
- **`93 tied`** — seeds where the two arms gave *exactly* the same number.
- **`significant`** / **`NOT significant`**.

**The tie count is the part people skip, and it is the most informative field.** A small
difference with a *high* tie count means the two arms are mostly making the same decision —
a different conclusion from "the effect is real but hard to see". When every arm ties on
every seed, the report says so explicitly, because "no significant effect" reads as
evidence of no effect when it is usually evidence that the dial does not reach the metric
in this scenario at all.

More seeds shrink the SE as `1/√n`: to halve the error bar, quadruple the seeds. If an
effect is not visible at 2,000 seeds it is small enough that the honest answer is usually
"smaller than anything else in this model".

---

## How fast, and why

Measured on this machine (12 threads), release build:

| Study | Trials | Wall time | Rate |
|---|---|---|---|
| `sweep ad_c2 --values 1,2,3,4 --seeds 2500` | 10,000 | 17.8 s | 562/s |
| `batch --only air_raid --seeds 2000` | 2,000 | 8.1 s | 248/s |
| `sweep fire_allocation --values ×3 --seeds 500` | 1,500 | 0.3 s | 4,715/s |

A trial costs microseconds; **building the terrain costs seconds**. So the seed list is cut
into exactly one chunk per worker thread, and each worker builds one sim and resets it
between trials. That is `threads` terrain builds, paid once and concurrently — not one per
trial, and deliberately not `rayon::map_init`, whose init closure is called an unspecified
number of times.

Every worker builds terrain from `scenario.default_seed`, so all workers get the **same**
map. Terrain generation is deterministic given its seed, so that is exact.

**Scheduling cannot change the answer.** Results come back in seed order regardless of how
the work was split, and each trial is a fresh `reset_to_scenario` whose RNG stream depends
only on its seed. A parallel study returns byte-identical numbers to a serial one, and
`study::tests::parallel_matches_serial_exactly` pins that — because "we parallelised the
study and the answer changed" is otherwise found out months later, by a confusing result.

The other lever is `--until`. Most scenarios have decided by 200–300 s and the rest of the
run is empty ticks; halving `--until` nearly halves the cost. Check `*_cleared_s` first to
see whether you are paying for time in which nothing happens.

---

## Worked example: is the overkill cap earning its keep?

`max_batteries_per_air_target` caps how many air-defence batteries may be assigned to one
airframe. It defaults to 2, on the reasoning that a second battery is insurance against the
first missing and a third is nearly always waste. That is a claim, so measure it.

```bash
cargo run -p experiments --release --bin sweep -- ad_c2 \
    --param sim.max_batteries_per_air_target --values 1,2,3,4 \
    --seeds 2500 --metric ad_rounds_left
```

10,000 trials, 17.8 s:

| cap | `air_downed` | `ad_rounds_left` |
|---|---|---|
| 1 | 9.907 (baseline) | 3.679 (baseline) |
| 2 | −0.002 ± 0.007 — **not significant** | −0.252 ± 0.040 (t = −6.3) |
| 3 | −0.028 ± 0.008 (t = −3.7) | −0.642 ± 0.042 (t = −15.2) |
| 4 | −0.028 | −0.650 — identical to cap 3 |

Read it in three parts.

**Cap 4 is exactly cap 3**, because the scenario has three batteries. That is the harness
confirming it is measuring what it claims to.

**The second battery buys nothing and costs a quarter of a round.** −0.002 kills against an
SE of 0.007 is a null result at 2,500 paired seeds — not "too small to see", genuinely
nothing.

**The third battery is actively worse**: it costs 0.64 rounds of reserve *and* kills 0.028
fewer drones (t = −3.7). Stacking is not free even when ammunition is not the binding
constraint, because a battery committed to an airframe another battery has already covered
is not covering a different one.

So on this scenario the default of 2 is defensible but unearned — it is not harmful, and it
is not doing anything either. Whether that holds when batteries are scarcer relative to the
raid is the next question, and it is one `--set` away.

### A stat-block dial: what is a better sensor worth?

The same command shape reaches the models themselves. `mast_optical` is the early-warning
sensor `air_raid` hangs on — it is what cues the SAM, and the SAM is Blue's only answer
above the CIWS ceiling. So: what does its detection rate buy?

```bash
cargo run -p experiments --release --bin sweep -- air_raid \
    --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 \
    --seeds 1000 --metric air_leakers
```

| λ₀ (per s) | leakers, paired against 0.05 |
|---|---|
| 0.05 | 0.703 (baseline) |
| 0.1 | −0.029 ± 0.023 — not significant |
| 0.35 | −0.087 ± 0.032 (t = −2.7) |
| 1.0 | −0.176 ± 0.031 (t = −5.6) |

A twentyfold better sensor stops a quarter of the leakers. Note the shape: the first
doubling is worth nothing measurable, and it takes a factor of seven before the effect
clears the noise. That is what `P = 1 − e^{−λΔt}` looks like from the outside — the sensor
is not the binding constraint until it is bad enough to be one, and after that each
increment matters less than the last.

---

## Adding a metric

Three edits, all in [`crates/experiments/src/outcome.rs`](../crates/experiments/src/outcome.rs):

1. A field on `Outcome`.
2. A name in `COLUMNS`.
3. A line in `Outcome::values()`, in the same position.
4. Bump `N`.

`COLUMNS` and `values()` are tied together by the array length `N`, so forgetting one of
them will not compile. Fill the field in `run_one` **from the sim's logs or final state** —
that is the rule that keeps the metrics honest.

Everything else picks it up automatically: both CSVs, the summary, and `--metric <name>`.

## Adding an experiment

For a question `sweep` cannot phrase, add a binary in `crates/experiments/src/bin/`. Use
the harness rather than reimplementing it:

```rust
use experiments::{csv, stats::paired, study::{column, run_study, StudyConfig}};

let cfg = StudyConfig { seeds: 1000, until_s: 600.0, progress: true };
let a = run_study(&scenario_a, &libs, cfg)?;
let b = run_study(&scenario_b, &libs, cfg)?;
let metric = experiments::Outcome::column("red_cleared_s").unwrap();
println!("{}", paired(&column(&a, metric), &column(&b, metric)).report());
```

Both arms ran seeds `0..1000`, so they are paired by construction. That is the point of
`StudyConfig::seeds` being a count rather than a range.

## The bespoke binaries

Older, single-purpose experiments, each answering one question its own way. Several predate
the shared harness and would be shorter written against it now.

| Binary | Question |
|---|---|
| `pd_sweep` | probability of detection against range |
| `duel_probe` | mutual-detection duel: who sees whom first |
| `sensor_siting` | where to put a sensor, by coverage |
| `risk_path` | least-risk path under a threat field |
| `interdiction` | the §6.3 sensing-vs-routing game equilibrium |
| `air_raid` | counter-air: cue latency vs leakers |
| `allocation_gap` | the §10.2 allocation solvers, paired |
| `bench`, `fires_bench` | tick, LOS, viewshed and fires cost |

`sweep fire_allocation --param sim.allocation` now reproduces `allocation_gap`'s headline
independently, which is a useful cross-check on both.

## What this harness cannot do yet

- **One dial at a time.** No factorial designs; `--set` pins the others. A 2-D grid is a
  shell loop over `sweep` for now.
- **No confidence intervals on quantiles**, only on means. A bimodal outcome averaged into a
  middle that never happens will not announce itself — plot the per-trial CSV.
- **`--seeds N` always means `0..N`**, so two studies at different `N` share a prefix rather
  than being independent. That is deliberate (it is what makes arms pairable), but it means
  "run 1,000 more seeds" is `--seeds 2000`, not a second run.
