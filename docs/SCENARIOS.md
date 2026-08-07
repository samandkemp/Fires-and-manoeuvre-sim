# Scenarios and stat blocks

How to add a new kind of thing, and how to build a situation to put it in. Everything here
is TOML in [`scenarios/`](../scenarios/); nothing here requires touching Rust.

Related: [`docs/OPERATIONS.md`](OPERATIONS.md) to run what you build,
[`docs/EXPERIMENTS.md`](EXPERIMENTS.md) to study it,
[`docs/design/`](design/) for the model each dial feeds.

**All numbers in this repository are abstract placeholder dials, not real munition or
sensor performance data.** The models are the product; these are the knobs.

## The two kinds of file

This split is the heart of the data model:

- **Libraries** — `units.toml`, `weapons.toml`, `sensors.toml`, `air.toml`,
  `air_defence.toml`, `c2.toml`, `terrain_types.toml` — say **what things are**. One entry
  per type.
- **Scenarios** — `default.toml`, `air_raid.toml`, `kill_chain.toml`, … — say **where
  things are**. Each placement names a `type` from a library.

So this, in `units.toml`:

```toml
[afv]
role = "armour"
height_m = 2.8
silhouette_width_m = 3.2
element_count = 3        # a troop of 3 vehicles
speed_m_s = 6.0
weapon = "afv_cannon"
[afv.signature]
optical = 0.75
```

is referred to by this, in any scenario:

```toml
[[red.units]]
id = "red-1"
type = "afv"
pos = [5600.0, 6400.0]
route = [[5600.0, 6400.0], [4300.0, 5200.0]]
```

Change `element_count` in the library and **every** `afv` in every scenario changes. That
is the point: numbers are dials you turn, never values baked into code.

A scenario is told apart from a library by *being parseable as a scenario* — it needs a
`name` and a `[terrain]` block, which no library has. There is no hard-coded list of
scenario names anywhere, so adding a library never confuses the app's picker or the batch
runner.

---

# Part 1 — Adding a type

## A unit

`units.toml`. A unit is **N sub-elements** that are removed one at a time, not a single
hit-pointed blob (see [§4.1](design/04-suppression-and-attrition.md)).

| Field | Meaning |
|---|---|
| `height_m` | Actor height for LOS and slant range |
| `silhouette_width_m` | Target width for the direct-fire hit integral |
| `element_count` | How many sub-elements. Drives fire volume *and* how much there is to kill |
| `speed_m_s` | Cross-country pace. `0.0` = emplaced |
| `weapon` | Optional key into `weapons.toml`. No weapon = unarmed |
| `[type.signature]` | Per-modality detectability, `[0,1]`. `optical` is implemented |
| `value` | Optional. What killing this is worth to the enemy's allocation. Omitted = derived |
| `role` | Optional. Free-form label a fire plan can sort on |

Two of these repay thought.

**`signature` is a table, not a number**, so adding acoustic or EO-IR sensing later means
adding a key (`acoustic = 0.8` for a vehicle with a generator running), not a schema
change.

**`value` is usually better left out.** Omitted, it is derived as `elements ×
weapon_threat`, where threat comes from rate of fire, kill probability and reach — so a
stat block with no score is still ranked sensibly by the allocator
([§10.2](design/10-the-decision-layer.md)). Declare it when you want to say something the
derivation cannot, like "the radar matters more than its firepower suggests".

### Worked example: adding a type

Say you want a light reconnaissance vehicle — fast, well-armed for its size, hard to see.

```toml
# units.toml
[recce_vehicle]
role = "recce"
height_m = 2.2
silhouette_width_m = 2.4
element_count = 2
speed_m_s = 9.0            # faster than the AFV troop
weapon = "afv_cannon"      # reuses an existing weapon
[recce_vehicle.signature]
optical = 0.45             # smaller than the 0.75 AFV
```

That is the whole change. It is now placeable in any scenario as `type = "recce_vehicle"`,
selectable in the app's unit dropdown, sweepable by `sweep --param
units.recce_vehicle.speed_m_s`, and it answers to `"recce"` in a priority list. No code, no
rebuild of anything but the scenario.

## A weapon

`weapons.toml`. Two classes, and the choice decides which model resolves the shot.

```toml
[afv_cannon]
class = "direct"           # LOS-gated, hit against the silhouette
rof_rounds_per_min = 6.0
max_range_m = 2500.0
dispersion_mrad = 0.5      # angular error; sigma(r) = dispersion * r / 1000
p_kill_given_hit = 0.7

[howitzer_155]
class = "indirect"         # ballistic; needs a track, not a sightline
rof_rounds_per_min = 3.0
max_range_m = 24000.0
min_range_m = 2000.0       # the inner dead zone
cep_m = 90.0               # circular error probable
lethal_radius_m = 40.0     # the Carleton kernel's scale
```

**Direct fire needs line of sight; indirect fire does not** — it needs a *track*, i.e.
somebody on your side has seen the target and the track has not lapsed. That asymmetry is
the whole difference between the two classes at the decision layer, and it is why an
artillery scenario needs an observer to be worth anything.

`min_range_m` is easy to forget and expensive to forget: place targets inside a howitzer's
dead zone and it will sit silent while you wonder what is broken.

### Anti-radiation munitions

```toml
[arm_missile]
class = "indirect"
anti_radiation = true
cep_m = 15.0               # against a transmitting radar: it homes
silent_cep_m = 120.0       # against a silent one: it does not
```

`cep_against(emitting)` is the single decision point, so every munition without the flag is
an exact identity ([§12.3](design/12-sead.md)). This is the dial that makes "go silent" a
real defensive option with a real cost.

## A sensor

`sensors.toml`.

```toml
[narrow_optical]
modality = "optical"       # the propagation channel
mount_height_m = 4.0       # ignored when the sensor is carried by a drone
max_range_m = 3000.0       # hard cutoff
lambda0_per_s = 0.6        # base glimpse rate at zero range
range_half_m = 1400.0      # the range at which the rate has halved
range_exponent = 2.0       # how sharply it falls off
for_width_deg = 70.0       # field of regard; omit for all-round
```

The detection rate is `λ = λ₀ · f(r) · σ · τ · (1 − c)` with `f(r) = 1/(1 + (r/r½)ⁿ)`, so
`lambda0_per_s` sets the ceiling and `range_half_m` sets where it collapses
([§3.2](design/03-sensing.md)).

**`for_width_deg` is what makes a sensor taskable.** A sensor with a finite field of regard
can only watch a slice of the map, so *where it points* is a decision — and it is the only
kind of sensor the tasking layer ([§10.3](design/10-the-decision-layer.md)) has anything to
do with. An all-round sensor has nothing to task.

## A drone

`air.toml`. A drone is an airframe plus, optionally, a sensor and/or a payload.

```toml
[strike_uas]
role = "strike"
height_m = 1.5
silhouette_width_m = 4.0
cruise_speed_m_s = 45.0
max_turn_rate_deg_s = 10.0    # implies r_min = v/omega = 258 m
endurance_s = 2400.0
payload = "guided_bomb"       # an `indirect` weapon from weapons.toml
munitions = 2
expendable = false            # a carrier: releases and flies on
release_range_m = 2500.0      # standoff distance
[strike_uas.signature]
optical = 0.35
```

- `sensor = "uas_optical"` instead makes it a recce platform — the sensor rides the
  airframe, seeing from its altitude and facing its heading.
- `munitions` + `expendable` span the spectrum in two dials: a reusable carrier drops
  several and flies on; a one-way attack munition carries one and dies with it.
- **`max_turn_rate_deg_s` is load-bearing, not decoration.** It is a rate limit on heading,
  so a drone asked to fly a 90° corner cannot — it flies an arc of radius `v/ω`, arriving
  late and displaced. That error is exactly what an air-defence engagement window is made
  of ([§9.2](design/09-air-and-counter-air.md)).
- `release_range_m` and the defending battery's `max_range_m` are a **matched pair**. Set
  release inside the gun's bubble and the drone must fly through it; set it outside and the
  long-range SAM and its cueing chain are what matter. This is the single most consequential
  number in a counter-air scenario.

## An air-defence battery

`air_defence.toml`. Two engagement models, because time-to-kill is distributed differently
in each ([§9.4](design/09-air-and-counter-air.md)).

```toml
[ciws]
role = "point_defence"
max_range_m = 2000.0
min_alt_m = 0.0
max_alt_m = 1500.0
mount_height_m = 3.0
requires_los = true
reaction_time_s = 2.0         # crew/system delay once a track is actionable
cue_latency_s = 4.0           # comms delay paid on any externally cued track
magazine = 30                 # 0 = unlimited
channels = 1                  # simultaneous engagements: what a raid saturates
sensor = "ciws_radar"         # organic radar: self-cueing, latency 0
[ciws.engagement.gun]
kill_rate_per_s = 0.9         # TTK ~ Exp(0.9), so E[TTK] = 1.1 s

[sam]
# ... envelope as above, but no `sensor`: dependent on the cueing chain, always pays
# cue_latency_s. That is the lever the air_raid experiment sweeps.
[sam.engagement.missile]
ssk_p = 0.65
missile_speed_m_s = 700.0
reload_s = 8.0                # E[TTK] = t_f/p + (1/p - 1) * reload_s
```

**A gun and a missile fail differently, not just at different rates.** A gun grinds
continuously (Poisson), so stacking two batteries on a target simply adds kill rates and
wastes nothing. A missile is a discrete round, so overkill is real. That is why
coordination pays for missiles and barely registers for guns — a result worth reproducing
before trusting any counter-air conclusion.

**Omitting `sensor` is a modelling statement**, not an oversight: a battery with no organic
radar can fire only on tracks handed to it over the net, so it always pays `cue_latency_s`.
An instance can also set `self_cue = false` to force that on a battery that *does* have a
radar.

## A C2 post

`c2.toml`. A post does not shoot, sense, or move. Its only effect is that air-defence
batteries within `coordination_range_m` allocate as one group.

```toml
[ad_command_post]
coordination_range_m = 6000.0
height_m = 4.0
silhouette_width_m = 5.0
link_latency_s = 0.0          # how long a battery must be inside before it is netted
[ad_command_post.signature]
optical = 0.85                # deliberately high
```

The signature is high on purpose. A command post concentrates antennas and vehicles, which
is what makes it findable — and **being findable is the point**, because it is the asset an
attacker most wants to kill first. Killing it costs no battery, no magazine, no envelope;
it costs only the coordination ([§11](design/11-command-and-control.md)).

## Terrain types

`terrain_types.toml`. Per-type dials that every derived layer reads.

| Field | Meaning |
|---|---|
| `feature_height_m` | Canopy or building height above ground — the blocking surface is `z + f` |
| `extinction_per_m` | κ, sight attenuation per metre of canopy: `τ = e^{−κL}` |
| `cover` | `[0,1]` protection against fires |
| `concealment` | `[0,1]` reduction in detectability |
| `mobility_cost` | `≥ 1` movement multiplier (`inf` = impassable) |

Trees attenuate (soft, `extinction_per_m = 0.08`); urban hard-blocks (`extinction_per_m =
0`, but `feature_height_m = 8.0`). Two different mechanisms, deliberately.

## Roles

Every stat block may carry a `role` — a free-form string. It exists for one purpose: a fire
plan can sort on it ([§13.1](design/13-the-kill-chain.md)).

Roles never mask classes. An entry in a priority list may name an **id** (`"red-cp"`), a
**role** (`"armour"`), a **class** (`unit`, `air_defence`, `c2`, `air`), or **`"all"`** —
and `"air_defence"` still matches a battery whose role is `"point_defence"`. So inventing a
role can only add precision, never take it away.

---

# Part 2 — Building a scenario

## The skeleton

```toml
name = "my_scenario"
default_seed = 11

[sim]
dt_s = 1.0
epoch_s = 10.0

[terrain]
cell_size_m = 10.0
width_cells = 700
height_cells = 300
[terrain.source.flat]
elevation_m = 0.0

[[blue.units]]
id = "gun-a"
type = "sp_gun"
pos = [800.0, 1500.0]

[[red.units]]
id = "tank-1"
type = "afv"
pos = [4800.0, 1250.0]
```

That is a complete, runnable scenario. `[sim]` and both forces are optional — every dial
has a default — so the minimum is a `name`, a `[terrain]` block and something to look at.

Positions are **world metres**, not cells. A 700 × 300 grid at 10 m/cell is 7 km × 3 km, so
`pos = [4800.0, 1500.0]` sits 4.8 km east and centred north–south.

## Describing the ground

`[terrain.source]` takes one of four forms. The first two are the originals; the last two
let a map be *described* rather than picked ([§1.3](design/01-terrain-and-los.md)).

```toml
[terrain.source.flat]                       # dead flat — isolates a model from terrain
elevation_m = 0.0

[terrain.source.hills]                      # seeded rolling relief
count = 24
max_height_m = 120.0
base_radius_m = 600.0
woods_fraction = 0.28
urban_blocks = 4

[terrain.source]                            # a named recipe
preset = "mountain_pass"                    # or rolling_hills, wooded_hills,
                                            # light_urban, dense_urban, flat_plain

[terrain.source.layers]                     # a recipe of your own
base = { hills = { count = 20, max_height_m = 90.0, base_radius_m = 700.0 } }
[[terrain.source.layers.apply]]
ridge = { bearing_deg = 20.0, crest_m = 320.0, width_m = 1400.0 }
[[terrain.source.layers.apply]]
woodland = { fraction = 0.32, patch_scale_m = 450.0 }
[[terrain.source.layers.apply]]
urban = { blocks = 5, min_size_m = 250.0, max_size_m = 500.0 }
```

Layers apply **in the order written** — urban after woodland leaves urban — and all draw
from one seeded stream, so a recipe plus a seed always reproduces the same map. `base` is
`flat` or `hills`; `apply` may be empty. `mountain_pass.toml` is a worked example.

The key inside `[[terrain.source.layers.apply]]` is **`apply`**, not `features`. Mistyping
it used to be silently ignored, producing a flat map where you thought you had a ridge; the
schema now rejects it.

Two forms deserve their reputations: **`flat` is the right choice for a validation
fixture** — it takes terrain out of the answer entirely, so anything you measure is the
model you meant to measure. And `preset` is the right choice when you want *a* map rather
than *this* map.

## Placing things

Every placement names an `id` (unique in the scenario), a `type` from a library, and a
`pos`. The extras differ:

```toml
[[blue.units]]
id = "red-1"
type = "afv"
pos = [5600.0, 6400.0]
route = [[5600.0, 6400.0], [4300.0, 5200.0]]   # optional; empty = static

[[blue.sensors]]
id = "obs-1"
type = "narrow_optical"
pos = [1000.0, 1500.0]
facing_deg = 90.0                              # 0 = east, CCW; needs a finite for_width_deg

[[blue.jammers]]
pos = [3000.0, 1500.0]
power = 0.8                                    # peak degradation at the centre, [0,1]
radius_m = 1200.0

[[red.air]]
id = "striker-1"
type = "strike_uas"
pos = [9000.0, 3000.0]
altitude_m = 300.0
altitude_ref = "agl"                           # or "amsl" — decides whether terrain masks it
heading_deg = 180.0
waypoints = [[6000.0, 3000.0], [4000.0, 3000.0]]
terminal = { orbit = { radius_m = 800.0, clockwise = true } }   # or "hold"
target = { unit = "sam-1" }                    # or { point = [x, y] }

[[red.air_defence]]
id = "ciws-1"
type = "ciws"
pos = [5200.0, 1500.0]
self_cue = true                                # false forces it onto the net

[[red.c2]]
id = "red-cp"
type = "ad_command_post"
pos = [5400.0, 1500.0]
```

`target = { unit = "..." }` resolves across **units, air-defence batteries and C2 posts** —
one namespace — so sending a strike drone at a SAM needs no special syntax
([§12.1](design/12-sead.md)). The key stayed `unit` for compatibility; `asset` is the
clearer alias and means the same.

**`altitude_ref` is the decision that decides whether terrain can mask a drone.** `agl`
follows the ground and rides over ridges; `amsl` holds a constant height above sea level
and gets masked by anything taller. Same number, opposite behaviour
([§9.1](design/09-air-and-counter-air.md)).

## The `[sim]` dials

Every dial has a default, so a scenario states only what it wants to change.

| Dial | Default | What it does |
|---|---|---|
| `dt_s` | 1.0 | Tick length — the continuous cadence ([§7.1](design/07-the-simulation-loop.md)) |
| `epoch_s` | 10.0 | Decision-epoch length — the discrete cadence |
| `suppression_radius_m` | 35.0 | A round landing this close is a near miss ([§4.3](design/04-suppression-and-attrition.md)) |
| `p_suppress` | 0.15 | Chance one near miss steps suppression up |
| `recover_per_s` | 0.05 | Rate of stepping back down |
| `suppressed_fire_factor` | 0.4 | Outgoing fire multiplier while Suppressed |
| `track_hold_s` | 45.0 | How long a track survives unobserved ([§10.1](design/10-the-decision-layer.md)) |
| `track_maintain_p` | 0.5 | How good a look must be to refresh a track |
| `allocation` | `optimal` | `optimal` / `greedy` / `independent` ([§10.2](design/10-the-decision-layer.md)) |
| `max_shooters_per_target` | 3 | Overkill cap: ground shooters per target per epoch |
| `max_batteries_per_air_target` | 2 | Overkill cap: air-defence batteries per airframe ([§11.2](design/11-command-and-control.md)) |
| `fires_need_c2` | `false` | Must a ground shooter be under a live C2 post to coordinate? ([§11.3](design/11-command-and-control.md)) |
| `sensor_tasking` | `false` | Do steerable sensors search by belief? ([§10.3](design/10-the-decision-layer.md)) |
| `belief_cells` | 48 | Edge length of the coarse belief grid |

Four are worth knowing as **switches back to older behaviour**, which is how one model is
isolated from another:

- `allocation = "independent"` restores the pre-Phase-10 rule where every shooter picked
  the nearest enemy for itself. Comparing it against `optimal` is what `allocation_gap`
  measures — and the answer is that coordinating is worth about 15% off the time to clear
  the enemy, while *optimality* over greedy is worth nothing measurable.
- `track_hold_s` set towards the run length recovers permanent detection — useful when you
  want to study fires without tracks lapsing underneath you.
- `sensor_tasking` is **off by default**, so a `facing_deg` you write is taken as meant.
  Turn it on to let sensors search, but note it dissolves any scenario whose premise is a
  *committed* sensor posture — the interdiction game being the case that caught this.
- `fires_need_c2` is **off by default**, so a side's guns coordinate for free. Turn it on
  and a shooter must be inside a live friendly post's radius to join the side-wide fire
  plan; one outside picks for itself. Turning it on unconditionally would have reduced every
  existing scenario to `independent` overnight, which is why it is a dial.

## Dials that live on stat blocks, not `[sim]`

Three matter for counter-air and SEAD, and they sit on the asset because they belong to the
thing rather than the situation:

| Dial | On | What it does |
|---|---|---|
| `link_latency_s` | `[c2.*]` | How long a battery must be inside the radius before it is netted. Default 0 |
| `value` | `[air.*]`, `[air_defence.*]`, `[c2.*]` | What destroying this is worth to the enemy's allocation. Emplacements get no cross-class derivation, so this is how "the SAM before the tanks" is said |
| `anti_radiation` + `silent_cep_m` | `[weapons.*]` | Homes on a transmitting radar; lands with `silent_cep_m` against a silent one |

There is **no dial for jamming a C2 link** — a jammer already does it. An enemy jammer near
a post scales its coordination radius by the EW factor, so batteries on the flanks fall out
of the net while the one on top of the post keeps talking. SEAD hard-kills the post; EW
soft-kills its reach ([§11.2](design/11-command-and-control.md)).

## The kill chain

A side **always** has a fire plan ([§13](design/13-the-kill-chain.md)). Omitting the block
gives `priority = ["all"]` — one tier holding everything, ranked by the ordinary payoff,
which *is* the undirected behaviour. Declare one and it is **followed**:

```toml
[blue.doctrine]
priority = ["red-cp", "air_defence", "armour"]   # id, class, role — all valid
mode = "strict"                                  # the default; or "weighted"

[[blue.orders]]                                  # bypass the decision entirely
shooter = "gun-a"
target = "red-cp"
```

`strict` means a shooter that can reach a higher tier takes it *even at a worse shot* — a
crew follows orders, not a kill-probability table. `weighted` scales value by tier instead,
so doctrine biases the optimisation without overriding it. The difference is exactly the
cost of directive control against optimal control, and it is measurable.

**A name matching nothing is a load error** listing what would have worked — because a tier
that silently matches nothing is a doctrine nobody is following.

Two rules stop a fire plan wasting ammunition:

- **Line of sight and range block a pairing.** A shooter whose top tier is masked by a
  ridge falls through to what it can actually engage rather than idling, and an `[[orders]]`
  entry lapses the same way while its target is unreachable, resuming when it reappears.
- **A shooter holds its target** until that target is dead or can no longer be engaged,
  rather than re-deciding every epoch and flip-flopping between two similar targets. A new
  order is the one thing that breaks a lock, and a held lock still consumes a slot, so the
  overkill cap cannot be bypassed.

`kill_chain.toml` is the worked example, with the measured result in its header comment.

## Seeds

Everything random comes from one seeded generator. Same scenario + same seed = the same
battle, down to the last round. `default_seed` sets it; the app's **seed** box and
`--seeds` on the batch runner override it.

One subtlety that decides what your experiment is actually measuring:

- **`Sim::new` derives both the terrain and the dice from the seed.** Looping it over seeds
  varies the map *and* the luck together, so the variance you measure is both mixed.
- **`Sim::reset_to_scenario` keeps the terrain and re-rolls only the dice.** That is what
  you want for "what happens on *this* map, on average", and it is what the batch runner
  uses — which is also why it is ~15× faster.

---

## Loading and checking

```
cargo run -p app -- my_scenario        # by bare name, resolved in scenarios/
cargo run -p app -- path/to/mine.toml  # or by path
```

An unknown name prints the ones that would have worked. The app's **scenario** picker lists
every parseable scenario in the folder and switches between them live.

**Unknown keys are rejected.** Nearly every dial has a default, so a misspelt one —
`track_hold` for `track_hold_s` — used to parse perfectly, take the default, and quietly
change what the scenario meant. Every schema now sets `deny_unknown_fields`, so the loader
refuses it and names the key. This caught a real bug during development: a mistyped terrain
key that silently produced a flat map.

You do not have to edit a file to try a different value:

```
cargo run -p experiments --release --bin sweep -- my_scenario \
    --param sim.track_hold_s --values 10,20,45,90 --seeds 500
```

`--param` patches any dotted path — scenario **or** stat-block library — into the TOML
before it is parsed, and the result goes through exactly the same validation as a file on
disk. See [`docs/EXPERIMENTS.md`](EXPERIMENTS.md).

## The bundled scenarios

| File | What it demonstrates |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
| `fire_allocation.toml` | Four shooters that can all reach all four targets — where the allocation rule actually matters |
| `sensor_search.toml` | Narrow-arc observers searching by belief (needs `sensor_tasking`) |
| `kill_chain.toml` | Directed targeting, and ground counter-battery |
| `ad_c2.toml` | Coordinated vs decentralised air defence — delete the `[[blue.c2]]` block to compare |
| `air_raid.toml` | The counter-air scenario: a drone raid vs self-cued and net-cued defences |
| `mountain_pass.toml` | A composable terrain recipe: rolling base + ridge + woodland + urban |
| `flat_range.toml` | A flat, featureless test range — isolates models from terrain effects |

And the libraries: `units.toml`, `weapons.toml`, `sensors.toml`, `terrain_types.toml`,
`air.toml`, `air_defence.toml`, `c2.toml`. The last three are **optional** — a scenario
directory without them loads with those libraries empty, so an older scenario set still
works.

The schema all of these parse into is
[`crates/sim_core/src/scenario.rs`](../crates/sim_core/src/scenario.rs), which is the place
to look when this document and the code disagree.
