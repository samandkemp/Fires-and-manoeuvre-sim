# How it works

A walkthrough for someone new to the codebase. It assumes no Rust and no operational
research, and it answers the practical questions: what is a scenario, how does a unit get
spotted, how does it get shot, and where does each number come from.

This is deliberately a different job from its neighbours:

| Doc | Question it answers |
|---|---|
| `README.md` | *Why* these methods? The six OR strands and the argument for each |
| `docs/DESIGN.md` | *What* is specified? Equations, invariants, and the gate for every model |
| **this file** | *How* does it actually work, and where is the code? |

Every number below was computed by the real functions, not by hand.

---

## 1. The shape of the code

Four crates. The arrows only point one way, and that is the single most important
structural fact about the project:

```
                 ┌──────────────┐
                 │   sim_core   │   the OR engine. No graphics, no window, no
                 │  (the maths) │   wall-clock. Deterministic given (scenario, seed).
                 └──────┬───────┘
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
   ┌─────────┐   ┌─────────────┐   ┌────────────┐
   │   app   │   │ experiments │   │ validation │
   │  (Bevy) │   │  (headless) │   │ (V1–V58)   │
   └─────────┘   └─────────────┘   └────────────┘
```

`sim_core` never depends on the app or on Bevy. That is what lets you run ten thousand
battles with no window open, and what lets the validation crate check the maths through
the public API alone.

### Inside `sim_core`

Each module owns one idea:

| Module | What it does |
|---|---|
| `terrain.rs` | The map: elevation raster, terrain types, and derived cover / concealment / mobility layers |
| `los.rs` | Line of sight. Walks the grid between two points and reports what it found |
| `sensing.rs` | The detection *rate* model — pure functions, no state |
| `fires.rs` | Hit probability and area damage — pure functions, no state |
| `suppression.rs` | The Free / Suppressed / Pinned state machine |
| `movement.rs` | Least-risk pathfinding |
| `air.rs`, `air_defence.rs` | Drones and the things that shoot them |
| `ew.rs`, `pomdp.rs` | Jamming, and reasoning about where an unseen enemy might be |
| `game.rs` | The zero-sum game solver |
| `allocation.rs` | Weapon–target assignment: who shoots what |
| `scenario.rs` | Loading TOML into structs |
| `sim/` | **The engine that drives all of the above** |

Notice the split: `sensing.rs` and `fires.rs` are *pure functions* — give them numbers,
they give you a probability, with no memory and no randomness. `sim/` is the part that
holds state, owns the dice, and calls them in the right order. That separation is why the
models can be validated in isolation.

### Inside `sim/`

```
sim/mod.rs          the Sim struct and step_one() — THE TICK. Start here.
sim/state.rs        what a placed unit / sensor / jammer is
sim/events.rs       the append-only logs of everything that happened
sim/setup.rs        building a Sim and placing assets into it
sim/commands.rs     what the app's mouse can change between ticks
sim/detection.rs    the glimpse process, EW, and the track lifecycle
sim/engagement.rs   ground fires: picking a target, resolving rounds
sim/counter_air.rs  the air phases
sim/tasking.rs      belief, and where each sensor should look next
sim/los_cache.rs    a speed-up (see §9); no effect on results
```

**If you read one function, read `Sim::step_one` in `sim/mod.rs`.** Everything else hangs
off it.

---

## 2. Interacting with scenarios

A scenario is a TOML file describing *a situation*: what the ground looks like and who is
standing on it.

### Running one

```
cargo run -p app                       # the `default` scenario
cargo run -p app -- air_raid           # by bare name, resolved in scenarios/
cargo run -p app -- path/to/mine.toml  # or by path
```

The app's **scenario** dropdown lists every file in `scenarios/` that parses as a
scenario and switches between them live. A scenario is told apart from a stat-block
library by *being parseable as one* — it needs a `name` and a `[terrain]` block, which
`units.toml` and friends do not have. So adding a new library never confuses the picker.

Headlessly, over many random seeds:

```
cargo run -p experiments --release --bin batch -- scenarios --seeds 50
```

That writes `out/<scenario>.csv` (a row per seed) and `out/summary.csv` (mean and
standard error per scenario).

### The two kinds of file

This split is the heart of the data model:

- **Libraries** (`units.toml`, `weapons.toml`, `sensors.toml`, `air.toml`,
  `air_defence.toml`, `terrain_types.toml`) say **what things are**. One entry per type.
- **Scenarios** (`default.toml`, `air_raid.toml`, …) say **where things are**. Each
  placement names a `type` from a library.

So this, in `units.toml`:

```toml
[afv]
height_m = 2.8
silhouette_width_m = 3.2
element_count = 3       # a troop of 3 vehicles
speed_m_s = 6.0
weapon = "afv_cannon"
[afv.signature]
optical = 0.75
```

is referred to by this, in a scenario:

```toml
[[red.units]]
id = "red-1"
type = "afv"
pos = [5600.0, 6400.0]
route = [[5600.0, 6400.0], [4300.0, 5200.0]]
```

Change `element_count` in the library and **every** `afv` in every scenario changes. That
is the point: the numbers are dials you turn, never values baked into code.

### Describing the ground

`[terrain.source]` takes one of four forms:

```toml
[terrain.source.flat]                 # dead flat — isolates a model from terrain
elevation_m = 0.0

[terrain.source.hills]                # seeded rolling relief
count = 24
max_height_m = 120.0
base_radius_m = 600.0
woods_fraction = 0.28
urban_blocks = 4

[terrain.source]                      # a named recipe
preset = "mountain_pass"

[terrain.source.layers]               # your own recipe: a base plus ordered layers
base = { hills = { count = 20, max_height_m = 90.0, base_radius_m = 700.0 } }
[[terrain.source.layers.apply]]
ridge = { bearing_deg = 20.0, crest_m = 320.0, width_m = 1400.0 }
[[terrain.source.layers.apply]]
woodland = { fraction = 0.32, patch_scale_m = 450.0 }
```

Layers apply **in the order written** — urban after woodland leaves urban — and all draw
from one seeded stream, so a recipe plus a seed always reproduces the same map.

### Seeds

Everything random comes from one seeded generator. Same scenario + same seed = the same
battle, down to the last round. `default_seed` in the file sets it; the app's **seed** box
and `--seeds` on the batch runner override it.

One subtlety worth knowing: `Sim::new` derives *both* the terrain and the dice from the
seed, so looping it over seeds varies the map and the luck together.
`Sim::reset_to_scenario` keeps the terrain and only re-rolls the dice — which is what you
want for "what happens on *this* map, on average", and what the batch runner uses.

---

## 3. What one tick does

`Sim::step_one` advances the clock by `dt_s` (default **1 second**) and runs eight
phases in a fixed order:

```
1. ground movement          units advance along their routes
2. air movement             drones fly; carried sensors move with them
3. sensing vs ground        ← detection happens here
4. sensing vs air
5. suppression recovery     suppressed units calm down
6. air defence              batteries engage drones
7. strike release           drones drop munitions
8. IF an epoch boundary was crossed:      ← all the DECIDING happens here
      maintain tracks       refresh or expire what is being watched
      task sensors          re-point steerable sensors by belief (if enabled)
      resolve fires         allocate targets side-wide, then shoot
```

Those three run in that order for a reason: tasking reasons about what was *not* seen
this epoch, so tracks must be settled first; and fires are gated on tracks, so allocation
comes last.

Two different clocks, and the distinction matters:

- **The tick** (`dt_s`, 1 s) integrates things that change continuously — movement, and
  the moment-to-moment chance of spotting something.
- **The epoch** (`epoch_s`, 10 s) is when *decisions* are made — who to shoot, what is
  still being tracked.

That split is deliberate. Real fire missions are not re-planned sixty times a minute, and
separating the two is what keeps the expensive decision logic off the hot path.

Phases 4, 6 and 7 do nothing at all — and, importantly, **consume no randomness** — in a
scenario with no aircraft. That is why adding the air model did not change the results of
any existing ground scenario.

---

## 4. How detection works

This is the centrepiece of the tool, so it gets the most space.

### The idea: a rate, not a chance

The naive approach would be "each second, roll a die with probability *p* of spotting". The
model does something better: it computes a **rate**, λ (lambda), in detections per second,
and treats spotting as a Poisson process.

```
P(spotted within t seconds) = 1 − e^(−λt)
```

Why bother? Because a rate is a property of the *situation*, whereas a per-tick
probability is a property of the situation **and your choice of tick size**. With a rate,
running at 0.5-second ticks gives statistically identical results to 1-second ticks. With
a per-tick probability, halving the tick would silently halve the detection rate — the
physics would depend on the integrator, which is the sort of bug that survives for years.

The per-tick draw is derived from the rate:

```rust
p_detect_tick(lambda, dt) = 1.0 - exp(-lambda * dt)
```

### The rate, factor by factor

`sensing::detection_rate_against` builds λ as a product:

```
λ = λ₀ · f(r) · signature · τ · (1 − concealment)
```

| Factor | Meaning | Where it comes from |
|---|---|---|
| `λ₀` | Peak rate: a perfect target at point-blank range | `lambda0_per_s` in `sensors.toml` |
| `f(r)` | Range falloff, `1 / (1 + (r/r½)^n)` | `range_half_m`, `range_exponent` |
| `signature` | How conspicuous this target is to this sensor | `[unit.signature]` per modality |
| `τ` | Canopy transmittance, `e^(−κL)` over `L` metres of foliage | computed by `los.rs` |
| `(1 − concealment)` | Cover from the terrain the target stands in | `terrain_types.toml` |

Before any of that, three gates can zero it outright (`sensing::detection_gate`):

1. **Range.** Beyond `max_range_m`, nothing.
2. **Field of regard.** Outside the sensor's arc, nothing. A sensor with no
   `for_width_deg` sees all round.
3. **Line of sight.** Hard-blocked by ground or a building, nothing.

Order matters here for speed: the range check is a couple of arithmetic operations, the
line-of-sight walk crosses the whole grid. So the cheap gates run first.

### Worked example

A `mast_optical` sensor (λ₀ = 0.35/s, r½ = 1800 m) watching an `afv`
(signature 0.75) at **2000 m**:

```
f(2000) = 1 / (1 + (2000/1800)²) = 0.4475
```

| Target situation | τ | concealment | λ (per s) | Mean time to spot | P(spotted in 10 s) |
|---|---|---|---|---|---|
| In the open | 1.0 | 0.0 | 0.1175 | **8.5 s** | 69% |
| Sitting in woods | 1.0 | 0.6 | 0.0470 | **21.3 s** | 37% |
| In woods, seen through 50 m of canopy | 0.018 | 0.6 | 0.00086 | **1162 s** | 0.9% |

Read the last row carefully, because it is the whole argument for modelling terrain
properly. Sitting in woods roughly **doubles** your survival time. Sitting in woods *with
foliage between you and the observer* multiplies it by **137**. Concealment and
transmittance are different things — where you stand versus what the sightline passes
through — and conflating them would lose that distinction entirely.

### The code path

```
sim/detection.rs :: detect_units()          for each (sensor, untracked enemy) pair
  └─ Sim::glimpse()                          one pair, one tick
       └─ Sim::effective_rate()
            ├─ sensing::detection_gate()     range / arc / signature → slant range
            ├─ Sim::cached_los()             → los::line_of_sight()
            ├─ sensing::rate_given_los()     the product above
            └─ Sim::jamming_at()             × the EW factor (§8)
       └─ if rng.random() < p_detect_tick(λ, dt) → detected
```

**One random draw per eligible pair per tick, in a fixed order.** That is the unit the
determinism guarantee is built on.

### Line of sight

`los::line_of_sight(terrain, a, h_a, b, h_b)` walks the grid between two points and
returns more than a yes/no:

```rust
LosResult {
    clear: bool,                // any hard block?
    transmittance: f32,         // τ — how much gets through the foliage
    mask_height: f32,           // how much taller the target would need to be to be seen
    blocked_at: Option<f32>,    // how far along the block occurred, if it did
    canopy_length: f32,         // metres of sightline under foliage
}
```

Three heights are kept strictly separate, and never conflated:

- **Ground elevation** `z` — the bare earth.
- **Feature height** `f` — trees and buildings *above* the ground. The blocking surface
  is `z + f`.
- **Actor height** `h` — eye or mast height above the ground you stand on.

So a unit in woods sits at `z + h`, **under** the canopy at `z + f` — it can see out from
beneath its own trees. Urban blocks hard; trees attenuate.

Range is always **slant range** — `√(horizontal² + Δheight²)` — never horizontal. A drone
at 400 m directly overhead is 400 m away, not 0 m.

---

## 5. Tracks: detection decays

Spotting something once does not mean you keep watching it. A track is **held** only while
it is refreshed, and lapses `track_hold_s` (default **45 s**) after the last observation.

At each epoch, `maintain_tracks` asks: would a sensor expect to glimpse this target again
during the next epoch?

```
refresh if   1 − e^(−λ_eff · epoch_s)  ≥  track_maintain_p     (default 0.5)
```

Two design points that are easy to miss:

**Maintenance is deterministic.** Acquiring a target is a dice roll; keeping your eyes on
something you have already found is not. No randomness is drawn here.

**It uses the *effective* rate**, jamming included. This is what allows electronic warfare
to *break* an existing track, not merely prevent a new one — jam a sensor hard enough and
λ_eff falls below the threshold, and the track ages out even with a clear view. Before
this existed, detection was permanent, and jamming an already-spotted unit did precisely
nothing.

---

## 6. How target engagement works

At each epoch, `sim/engagement.rs :: resolve_fires` runs every live unit through four
steps.

### Step 1 — can it shoot?

Skipped if dead, if **Pinned** (suppression), or if it carries no weapon.

### Step 2 — allocate targets (`allocate_fires`)

Each side assigns **all** its shooters at once, rather than each choosing for itself. The
payoff for putting shooter `i` on the `k`-th slot of target `j` is

```
q(i,j) × value(j) × (1 − q̄(j))^k
```

`q` is the fraction of the target this shooter expects to destroy this epoch (straight
from the fires model below), `value` is what the target is worth, and the last term is
diminishing returns — a second shooter on a target only helps if the first failed.
Solved optimally by the Hungarian algorithm.

Why bother? Because the obvious rule wastes fire in an obvious way: three tanks all
engage the nearest enemy while a second, equally dangerous one is untouched. Set
`[sim] allocation = "independent"` to get that old behaviour back and compare.

Eligibility differs by weapon class, and the difference is the point:

| | Direct fire | Indirect fire |
|---|---|---|
| Needs line of sight? | **Yes** | No — it arcs over |
| Needs a track? | **No** | **Yes** |
| Rationale | You shoot what you can see | You bombard where you have been *cued* |

That asymmetry is why sensing matters. Artillery cannot fire at what nobody is watching,
so a sensor that loses its track silences the guns behind it.

### Step 3 — work out the shot once (`prepare_shot`)

Range, hit probability and dispersion depend only on shooter, target and weapon — none of
which change during the burst. So they are computed **once**, and the round loop only
rolls dice. (This is worth roughly a factor of two on the fires path.)

### Step 4 — fire the rounds

```
rounds this epoch = round(rof_rounds_per_min × epoch_s / 60) × live elements
```

Note **× live elements**. A unit is not a point; it is *N* sub-elements (3 vehicles in a
troop, 8 dismounts in a section), and each one shoots. Lose half your strength and you
lose half your output — which is precisely what makes an aimed-fire duel reproduce
**Lanchester's square law**, the strongest single check on the whole attrition chain.

---

## 7. How probability of kill is determined

### Direct fire

The round scatters as a 2-D Gaussian about the aim point. Angular dispersion becomes
linear spread with range:

```
σ(r) = dispersion_mrad × r / 1000        metres
```

The target is a rectangle — `silhouette_width_m` wide, `height_m` tall. Because horizontal
and vertical aiming errors are independent, hitting it is a product of two one-dimensional
Gaussian integrals:

```
P_hit = erf( W / (2σ√2) ) · erf( H / (2σ√2) )
P_kill = P_hit × p_kill_given_hit × (1 − cover) × suppression_factor
```

For an `afv_cannon` (0.5 mrad, `p_kill_given_hit` 0.7) against an `afv` (3.2 m × 2.8 m):

| Range | σ | P_hit | P_kill in the open | P_kill in urban (cover 0.7) |
|---|---|---|---|---|
| 500 m | 0.25 m | 1.000 | **0.700** | 0.210 |
| 1500 m | 0.75 m | 0.907 | **0.635** | 0.191 |
| 2500 m | 1.25 m | 0.589 | **0.413** | 0.124 |

Cover is doing enormous work here: at 1500 m, being in a built-up area cuts lethality by
**70%** — a bigger effect than tripling the range.

### Indirect fire

Two stages. First, where does the round land?

```
burst = aim + Gaussian noise,   σ = CEP / 1.1774
```

`CEP` (circular error probable) is the radius containing half the rounds; the constant is
`√(2 ln 2)`, the circular-Gaussian identity relating the two.

Second, what does a burst do to something `ρ` metres away? The **Carleton** damage kernel:

```
D(ρ) = exp( −ρ² / 2R_L² )
```

`R_L` is the lethal radius. Each surviving element then rolls independently against
`D × (1 − cover)`, so area fire attrits a group properly rather than killing all or none.

The expected damage, averaged over where the round actually lands, has a **closed form** —
a Gaussian convolved with a Gaussian:

```
E[D](d) = R_L²/(σ² + R_L²) · exp( −d² / 2(σ² + R_L²) )
```

That closed form is *why this kernel was chosen*. A simpler cookie-cutter lethality disc
would be cheaper, but it has no analytical expectation, so there would be nothing to check
the sampler against — and an unvalidated sampler is exactly what this project exists to
avoid.

For a `howitzer_155` (CEP 90 m → σ = 76.4 m, `R_L` = 40 m):

| Aim offset | E[D] per round | Expected casualties/round on a 3-element unit |
|---|---|---|
| 0 m (dead on) | 0.215 | 0.65 |
| 50 m | 0.182 | 0.55 |
| 100 m | 0.110 | 0.33 |
| 200 m | 0.015 | 0.04 |

Even a perfectly aimed 155 mm round expects to kill only **0.2 of an element**, because a
90 m CEP is large next to a 40 m lethal radius — most rounds land too far away to matter.
Artillery works by volume, and the model says so without being told to.

Contrast a `guided_bomb` (CEP 12 m, `R_L` 45 m): `E[D](0) = 0.951`. Precision changes the
kill mechanism entirely — one munition instead of a fire mission. Both use the *same*
code path; only `cep_m` differs.

---

## 8. Suppression, EW, and everything else

**Suppression** is a three-state machine per unit: `Free → Suppressed → Pinned`. Near
misses (a round landing within `suppression_radius_m`, default 35 m, that does *not* kill)
push the state up with probability `p_suppress` (0.15). Time pushes it back down at
`recover_per_s` (0.05/s). Effects:

- **Suppressed** — fire effectiveness × `suppressed_fire_factor` (0.4). Can still move.
- **Pinned** — cannot fire, cannot move.

This is what lets fires *shape* manoeuvre without killing anybody, which is most of what
artillery is actually for.

**Electronic warfare** enters as a multiplier on the detection rate, nothing else:

```
λ_eff = λ × Π_jammers (1 − power·(1 − d/radius))
```

With no jammers every factor is exactly 1, so switching EW off reduces to the plain
sensing model bit-for-bit — an identity, not an approximation.

**Belief** (`pomdp.rs`) answers "given we have seen nothing, where could they be?" The
key move is that *not* seeing something is evidence: a cell your sensor covers well has a
low likelihood of producing no detection, so belief drains out of it and pools in dead
ground and jammed areas. The app's belief overlay draws exactly this.

---

## 9. Performance, and why it does not affect results

Two optimisations are worth knowing about because you will see them in the code.

**The shot is prepared once per burst**, not per round (§6, step 3).

**Line of sight is memoised** (`sim/los_cache.rs`). The sensing loop re-tests every
(sensor, untracked target) pair every tick, and each test walks the grid at ~77 µs. A unit
hidden behind a ridge gets re-walked by every sensor, every tick, forever — always for the
same answer. Most endpoints never move: emplaced guns and mast sensors sit still all
battle, and terrain never changes mid-run.

The cache reuses a result **only when both positions and both heights are exactly equal**
— no tolerance, no staleness window. A hit is therefore precisely the value a miss would
have computed. That is why this is a speed-up and not a model change, and it takes the
tick from ~105 µs to ~10 µs.

Run `cargo run -p experiments --release --bin bench` to see the current figures, including
the cache hit rate.

---

## 10. Where to change things

| You want to… | Edit |
|---|---|
| Make a sensor see further | `sensors.toml`: `max_range_m`, `range_half_m`, `lambda0_per_s` |
| Make a unit harder to spot | `units.toml`: `[type.signature]` |
| Make woods thicker | `terrain_types.toml`: `extinction_per_m`, `concealment` |
| Make a weapon more accurate | `weapons.toml`: `dispersion_mrad` (direct) or `cep_m` (indirect) |
| Make artillery more lethal | `weapons.toml`: `lethal_radius_m` |
| Change how long tracks last | scenario `[sim]`: `track_hold_s` |
| Make suppression stickier | scenario `[sim]`: `p_suppress` ↑, `recover_per_s` ↓ |
| Build a different map | scenario `[terrain.source]` — see §2 |
| Change how targets are chosen | scenario `[sim]`: `allocation` = `optimal` / `greedy` / `independent` |
| Let sensors search for themselves | scenario `[sim]`: `sensor_tasking = true` (needs a sensor with `for_width_deg`) |
| Change the *allocation payoff* | `sim/engagement.rs :: allocate_fires` — this is code, not a dial |
| Change the detection *model* | `sensing.rs` — and expect to update a validation gate |

The rule: if it is a number, it lives in TOML. If it is a decision or a functional form,
it lives in code — and code changes come with a gate.

---

## 11. Checking you have not broken anything

```
cargo test --workspace                                       # everything
cargo run -p validation --release --bin validation_report    # the gate table
cargo clippy --workspace                                     # lints
```

The **V-gates** (V1–V58) are the project's backbone. Each one checks a model against a
closed-form result or a stated invariant — not against a previously recorded output. The
difference matters: a regression test tells you the answer changed, while a gate tells you
the answer is *wrong*.

Examples:

- **V15** — Monte Carlo detection frequency against `1 − e^(−λt)`
- **V22** — sampled area damage against the Carleton–Gaussian closed form
- **V30** — attrition against Lanchester's square law
- **V40** — EW switched off is exactly the identity
- **V55** — a track lapses without observation, so jamming can break one
- **V56** — the allocator matches an exhaustive optimum, and never picks a forbidden pairing
- **V57** — belief-driven tasking finds enemies a fixed stare never does

`validation_report` prints each gate beside the closed form it is checked against, because
the useful question is not "are the tests green" but *is the maths still right, and right
against what*.

If you change a model and a gate fails, **understand why before re-baselining it**. That
gate is the only thing standing between a model and a plausible-looking number that is
quietly wrong.

---

## Where to go next

- `docs/DESIGN.md` — the full specification, with every equation and its gate
- `README.md` — the OR strands and why each was chosen
- `scenarios/README.md` — the scenario and stat-block file formats
- `SETUP.md` — environment setup, written for a Rust beginner
