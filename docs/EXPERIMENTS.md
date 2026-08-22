# Running experiments

How to get a number out of this simulation that can be defended.

The app shows one battle. That is the wrong tool for a question, because one battle is one
draw from a distribution and it is not the interesting one. This document is about the
other half: running the same situation thousands of times and reporting what actually
changed, with an error bar.

This is the **study-design** document - why paired, what the columns mean, how to read the
output, and what the harness cannot yet do. For the bare command syntax see
[`docs/OPERATIONS.md`](OPERATIONS.md); for the dials themselves see
[`docs/SCENARIOS.md`](SCENARIOS.md).

- [The five-minute version](#the-five-minute-version)
- [The two rules](#the-two-rules)
- [`batch` - a folder of scenarios](#batch--a-folder-of-scenarios)
- [`sweep` - one dial, many values](#sweep--one-dial-many-values)
- [`factorial` - several dials at once](#factorial--several-dials-at-once-and-whether-they-interact)
- [`sensitivity` - which dials drive the answer at all?](#sensitivity--which-dials-drive-the-answer-at-all)
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
  sim.allocation = independent  baseline 75.355
  sim.allocation = greedy      -12.835 +- 0.224 (t = -57.2, n = 2000, 311 tied) significant
  sim.allocation = optimal     -12.430 +- 0.231 (t = -53.8, n = 2000, 323 tied) significant
```

Read it as: coordinating fires clears the enemy 12.8 s sooner, and that is far outside the
noise. Solving the assignment *optimally* rather than greedily is worth 0.12 s, which is a
quarter of one standard error - nothing. Both arms gave literally the same answer on about
90 of the 500 seeds.

Always `--release`. A debug build of a Monte Carlo study is roughly 20× slower and tells
nothing a release build does not.

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

To average over maps - a fair question, just a different one - sweep
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

## `batch` - a folder of scenarios

```
batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]
```

| Flag | Default | What it does |
|---|---|---|
| `dir` | `scenarios` | folder of scenarios **and** stat-block libraries |
| `--seeds` | 20 | trials per scenario; seeds `0..N` |
| `--until` | 600 | sim seconds per trial |
| `--out` | `out` | where the CSVs go |
| `--only` | - | just this scenario, by bare name |
| `--quiet` | - | no progress line |

Writes `out/<scenario>.csv` (a row per seed, every metric) and `out/summary.csv` (a row per
scenario, mean and SE for every metric). `.gitignore` already covers `out/` and `*.csv`.

`batch` compares **scenarios**. It is the regression sweep - run it after a change to see
whether anything moved that should not have. To compare **dials**, use `sweep`.

## `sweep` - one dial, many values

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

A dial that is absent from the file - most of them, since nearly all have a default - is
created. That is safe because the schema sets `deny_unknown_fields` throughout, scenario
and stat blocks alike: `sim.track_hold` for `sim.track_hold_s` is a load error naming the
key, not a silent default. (Which also means a typo in a hand-written scenario or stat
block is now caught, instead of quietly changing what it means.)

`--set` is repeatable and applies to **every** arm, for holding one dial away from the
scenario's own setting while varying another.

`--metric` chooses which column the paired report is about; every column is still in the
CSV. Output goes to `out/<scenario>_<param>.csv` and `..._summary.csv`.

---

## `factorial` - several dials at once, and whether they interact

`sweep` answers *what does this dial do*. `factorial` answers *what do these dials do, and
does either one's answer depend on the other* - which is a different question, and on this
model it has more than once been the more important one.

```
factorial <scenario> --factor PATH=v1,v2 [--factor PATH=v1,v2]...
                     [--seeds N] [--until SECONDS] [--metric NAME] [--set PATH=VALUE]...
                     [--dir DIR] [--out DIR] [--quiet]
```

Every combination of levels is a **cell**, and every cell runs the same seed set. Cost is
the product of the level counts: three two-level factors is eight cells.

```
cargo run -p experiments --release --bin factorial -- fires_c2 \
    --factor sim.fires_need_c2=false,true \
    --factor blue.sensors.0.type=mast_optical,ciws_radar \
    --seeds 500 --until 300 --metric red_cleared_s
```

### What it reports

**Main effects** first, each averaged over every level of the other factors - so a factor is
described by what it does across the design, not at one corner of it.

**Interactions** second, as the classic difference of differences, formed per seed:

$$
(y_{11} - y_{01}) - (y_{10} - y_{00})
$$

for factors at their first and last levels, with any other factors averaged out. Zero means
the two dials are additive and can be reasoned about separately. Non-zero means they cannot.

The closing line says whether any interaction is significant, because **that decides whether
the main effects above may be read on their own**. If two dials interact, "this one is worth
−11 s" is a sentence with a missing clause.

### Why this exists

The `fires_c2` investigation needed a 2×2 over the overkill cap and how fast targets were
acquired. It was hand-stitched from four separate `sweep` runs, and the *interaction* turned
out to be the dominant effect: the cap mattered enormously when targets were scarce and
hardly at all when they were not. Two main effects would have described neither case.

That investigation ended in the cap being removed (§11.4, gate V68), and the same 2×2 run
through this tool is now the check that it worked:

```
--- red_cleared_s: main effects, averaged over the other factors ---
  sim.fires_need_c2 (baseline false)
      = true              +0.100 +- 0.604 (t = 0.2, n = 200, 101 tied) NOT significant
  blue.sensors.0.type (baseline mast_optical)
      = ciws_radar        -26.050 +- 2.156 (t = -12.1, n = 200, 18 tied) significant

--- red_cleared_s: two-way interactions, across each factor's range ---
  sim.fires_need_c2 x blue.sensors.0.type
      +0.600 +- 1.065 (t = 0.6, n = 200, 103 tied) NOT significant
```

The interaction that used to dominate is gone. What remains is a large, clean, *additive*
effect of seeing sooner - which is what one would expect of a model that no longer has an
artefact in it.

### Multi-level factors

A factor may have more than two levels. Main effects are reported per level against the
baseline; the interaction is the corner-to-corner contrast across each factor's **range**,
which is a summary rather than the whole surface. The per-cell CSV holds the rest.

## `sensitivity` - which dials drive the answer at all?

Every number in this repository is an **abstract placeholder**. That is deliberate and said
everywhere, but it leaves one question over every finding: *does it matter that the numbers
are invented?*

Neither `sweep` nor `factorial` can answer it. Both vary a few dials with the rest pinned
wherever the scenario happened to leave them, so both measure a slice through a space they
never explore. Global sensitivity analysis explores the space and reports what share of the
outcome's variance each dial is responsible for.

```
cargo run -p experiments --release --bin sensitivity -- studies/sensing.toml --seeds 20
```

The dial space is a **file**, not a pile of flags, because it is a design - something to
commit, review and re-run. See [`studies/README.md`](../studies/README.md) for the format.

### What comes back

**Morris** first, because it is cheap: `mu*` ranks dials by how much they move the answer,
`sigma` flags one whose effect depends on where the others are. Its job is to say what can be
ignored before the expensive pass runs.

**Sobol** second, as a variance decomposition:

| Column | Meaning |
|---|---|
| `S1` | the share of variance this dial explains **alone** |
| `ST` | the share it is involved in altogether, interactions included |
| `ST − S1` | the share running **through** interactions - invisible to a one-dial sweep |

The closing line adds the first-order indices up. Near 1 means the dials are additive and
one-at-a-time sweeps are sound. Well below 1 means most of the variance lives in
interactions, and a sweep will mislead.

### A worked result

> The model side is [§9.5](design/09-air-and-counter-air.md), the cueing timeline. As above,
> both carry the numbers.

`studies/sensing.toml` asks what decides whether a drone raid gets through - 32,720 trials
over four dials:

| dial | S1 | ST | ST − S1 |
|---|---|---|---|
| `air.strike_uas.cruise_speed_m_s` | 0.653 | 0.801 | 0.148 |
| `air_defence.sam.cue_latency_s` | 0.339 | 0.374 | 0.035 |
| `sensors.mast_optical.lambda0_per_s` | −0.001 | 0.057 | 0.058 |
| `sim.track_hold_s` | 0.000 | 0.000 | 0.000 |

**Raid speed dominates** - how fast the attacker crosses the envelope explains more than the
defender's cue latency does. **The sensor barely matters**, which retrospectively explains
why sweeping its glimpse rate over a 20× range moved leakage by only 0.176: it is not the
binding constraint here. **`track_hold_s` is exactly inert**, because the engagement resolves
faster than the shortest hold time in the range. And the first-order total of **0.990** says
the dials are additive on this scenario, which is a licence for every earlier `air_raid`
sweep.

A slightly negative `S1` means "indistinguishable from zero" - the Saltelli estimator is
unbiased rather than non-negative, and clamping it would hide how noisy a near-zero index is.

### Cost

`(morris points + sobol points) × seeds`, and `--seeds` is deliberately modest: a design
point is an **average over seeds**, and the variance being decomposed is the one across the
*dial space*, not across the dice. The study above is ~13 minutes.

Terrain is built once for the whole design rather than once per point - `study::run_design`
exists for exactly that, and without it the study above spends its entire runtime generating
1000×1000 rasters.

### The estimator is gated

**V71** checks the Sobol implementation against the analytic indices of the Ishigami
function, whose third input has a first-order index of *exactly zero* and a large total one.
An instrument that cannot recover a known answer cannot be trusted with an unknown one.

## What the columns mean

Every metric is read back from the sim's own event logs and final state - never accumulated
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
| `ad_shots` | air-defence shots taken - the denominator for rounds-per-kill |
| `ad_rounds_left` | interceptors remaining, finite magazines only |
| `ad_batteries_killed`, `c2_posts_killed` | what SEAD is trying to achieve |
| `blue_cleared_s`, `red_cleared_s` | when a side lost its last ground element |
| `epochs` | decision epochs resolved |

Two that repay attention:

**`*_cleared_s` is usually the metric that answers "was this better?"** Losses saturate: if
everything on one side dies by 600 s in every arm, `red_losses` is the same number
everywhere and only the *time* distinguishes them. That is how the Phase 10 allocation
result had to be measured. A side that was never cleared reports the run length, so read it
with the kill counts beside it - `600` means "not by 600 s", not "at 600 s". A side that
fields no ground units at all also reports the run length; there was nothing to clear.

**`ad_rounds_left` is where the Phase 11 C2 result lives.** Coordinating air defence barely
changes how many drones die; it changes how much ammunition is left afterwards, because a
missile is a discrete round and overkill is real. A gun is a Poisson process, so stacking
guns simply adds kill rates and wastes nothing. Coordination pays where the shot is a
countable resource - and that is invisible without this column.

---

## Reading the output

```
sim.allocation = greedy      -12.835 +- 0.224 (t = -57.2, n = 2000, 311 tied) significant
```

- **`-12.835`** - the mean paired difference against the *first* arm, in the metric's units.
- **`+- 0.224`** - its standard error. Roughly: the true value is within about 2 of these.
- **`t = -57.2`** - `mean / SE`. `|t| > 2` is the line this harness calls significant
  (two-sided, ~5%). At 57 there is nothing to argue about.
- **`311 tied`** - seeds where the two arms gave *exactly* the same number.

> **Every figure is against the first arm.** To compare two *other* arms, re-run with one of
> them first - do not eyeball the difference between two rows. Their separate errors do not
> combine the way the paired one does, and on this very example that mistake hid a real
> effect for two phases: greedy and optimal differ by 0.405 ± 0.051 measured directly, but
> read off their shared baseline the gap looks like 0.4 against SEs of ~0.23.
- **`significant`** / **`NOT significant`**.

**The tie count is the part people skip, and it is the most informative field.** A small
difference with a *high* tie count means the two arms are mostly making the same decision -
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
between trials. That is `threads` terrain builds, paid once and concurrently - not one per
trial, and deliberately not `rayon::map_init`, whose init closure is called an unspecified
number of times.

Every worker builds terrain from `scenario.default_seed`, so all workers get the **same**
map. Terrain generation is deterministic given its seed, so that is exact.

**Scheduling cannot change the answer.** Results come back in seed order regardless of how
the work was split, and each trial is a fresh `reset_to_scenario` whose RNG stream depends
only on its seed. A parallel study returns byte-identical numbers to a serial one, and
`study::tests::parallel_matches_serial_exactly` pins that - because "the study was parallelised and the
study and the answer changed" is otherwise found out months later, by a confusing result.

The other lever is `--until`. Most scenarios have decided by 200-300 s and the rest of the
run is empty ticks; halving `--until` nearly halves the cost. Check `*_cleared_s` first to
see whether time is being spent on nothing happening.

---

## Worked example: is the overkill cap earning its keep?

> The model side of this - *why* the dial exists and what it says about guns versus missiles
> - is [§11.2](design/11-command-and-control.md). This page is about the method. Both carry
> the numbers, so re-measuring means updating both.

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
| 2 | −0.002 ± 0.007 - **not significant** | −0.252 ± 0.040 (t = −6.3) |
| 3 | −0.028 ± 0.008 (t = −3.7) | −0.642 ± 0.042 (t = −15.2) |
| 4 | −0.028 | −0.650 - identical to cap 3 |

Read it in three parts.

**Cap 4 is exactly cap 3**, because the scenario has three batteries. That is the harness
confirming it is measuring what it claims to.

**The second battery buys nothing and costs a quarter of a round.** −0.002 kills against an
SE of 0.007 is a null result at 2,500 paired seeds - not "too small to see", genuinely
nothing.

**The third battery is actively worse**: it costs 0.64 rounds of reserve *and* kills 0.028
fewer drones (t = −3.7). Stacking is not free even when ammunition is not the binding
constraint, because a battery committed to an airframe another battery has already covered
is not covering a different one.

So on this scenario the default of 2 is defensible but unearned - it is not harmful, and it
is not doing anything either. Whether that holds when batteries are scarcer relative to the
raid is the next question, and it is one `--set` away.

### A stat-block dial: what is a better sensor worth?

The same command shape reaches the models themselves. `mast_optical` is the early-warning
sensor `air_raid` hangs on - it is what cues the SAM, and the SAM is Blue's only answer
above the CIWS ceiling. So: what does its detection rate buy?

```bash
cargo run -p experiments --release --bin sweep -- air_raid \
    --param sensors.mast_optical.lambda0_per_s --values 0.05,0.1,0.35,1.0 \
    --seeds 1000 --metric air_leakers
```

| λ₀ (per s) | leakers, paired against 0.05 |
|---|---|
| 0.05 | 0.703 (baseline) |
| 0.1 | −0.029 ± 0.023 - not significant |
| 0.35 | −0.087 ± 0.032 (t = −2.7) |
| 1.0 | −0.176 ± 0.031 (t = −5.6) |

A twentyfold better sensor stops a quarter of the leakers. Note the shape: the first
doubling is worth nothing measurable, and it takes a factor of seven before the effect
clears the noise. That is what `P = 1 − e^{−λΔt}` looks like from the outside - the sensor
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
them will not compile. Fill the field in `run_one` **from the sim's logs or final state** -
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

Four single-purpose experiments that do something the general tools cannot, plus the
benches. Each answers one question its own way and prints a table.

| Binary | Question | Why not a sweep |
|---|---|---|
| `duel_probe` | mutual-detection duel: who sees whom first | a diagnostic - prints per-pair geometry, not a metric |
| `sensor_siting` | where to put a sensor, by coverage | searches over *positions*, which is not a dial |
| `interdiction` | the §6.3 sensing-vs-routing game equilibrium | solves a game; there is no arm to compare |
| `air_raid` | counter-air: cue latency vs leakers | reports a closed form beside the measurement |
| `bench`, `fires_bench` | tick, LOS, viewshed and fires cost | performance, not behaviour |

**Three were deleted once the harness subsumed them** - `pd_sweep` (the sensing gates check
the same closed form, and `validation_report` prints it), `allocation_gap` (`sweep --param
sim.allocation --values optimal,greedy,independent` is the same comparison, paired, with
standard errors) and `risk_path` (§10.5 put least-risk pathing *in the loop*, and V73 gates
it). A demo that the engine has since absorbed is a maintenance cost, not a feature.

## Findings have to be re-run, not just recorded

A measured finding is a claim about a model at a moment. The model then changes, and unless
something re-runs the claim, a number that was right when written goes on being quoted after
it stopped being true.

[`findings.toml`](../findings.toml) pins each documented claim to the paired comparison that
produced it - scenario, dial, two arms, metric, seeds, the expected difference and a
tolerance - and `findings` re-runs them all:

```
cargo run -p experiments --release --bin findings
```

```
allocation-coordination-pays          holds  measured -12.835 +- 0.224, documented -12.835
allocation-optimal-is-worse           holds  measured +0.405 +- 0.051, documented +0.405
ad-overkill-cap-second-battery        holds  measured -0.002 +- 0.007, documented -0.002
ad-overkill-cap-third-battery         holds  measured -0.028 +- 0.008, documented -0.028
```

**The tolerance is not a confidence interval.** The run computes its own standard error. The
tolerance is the author's statement of how far the number may move before the prose around it
stops being true, which is a different and more useful question.

**Arms are compared against each other, never each against a shared baseline.** Reading a
difference across two baselines overstates its error about fivefold. A drift report also
lists every document repeating the number, because fixing a stale finding is mostly a matter
of finding all the places it was copied to.

## What this harness cannot do yet

- **Two-way interactions only.** `factorial` reports every pair of factors; a three-way
  interaction is in the per-cell CSV but not in the report. With more than two levels the
  reported interaction is the corner-to-corner contrast rather than the whole surface.
- **Sensitivity dials are continuous only.** A range is a pair of numbers, so a categorical
  dial like `sim.allocation` has no place in a study file - use `factorial` for those.
- **A sensitivity study may not vary terrain.** Terrain is built once for the whole design,
  so a terrain dial would ask for a map it does not get.
- **`--seeds N` always means `0..N`**, so two studies at different `N` share a prefix rather
  than being independent. That is deliberate (it is what makes arms pairable), but it means
  "run 1,000 more seeds" is `--seeds 2000`, not a second run.
