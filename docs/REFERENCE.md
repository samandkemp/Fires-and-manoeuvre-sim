# Reference

Every field, dial, metric and flag, with its type, default and the theory section that
specifies it. This is a lookup table: keep it open while editing a scenario. For the prose
walkthrough of *how* to use any of it, see [`docs/GUIDE.md`](GUIDE.md); for what a model does
with a dial, follow the § reference into [`docs/THEORY.md`](THEORY.md).

> Every value shipped in this repository is an **abstract placeholder**, not real munition or
> sensor performance data.

**Schema source of truth.** When this page and the code disagree, the code is right:
[`crates/sim_core/src/scenario/mod.rs`](../crates/sim_core/src/scenario/mod.rs) holds the
scenario schema, and each library type sits beside its model: `sensing.rs`, `weapon_effects.rs`,
`airframes.rs`, `air_defence.rs`, `c2.rs`, `terrain.rs`.

**Every schema sets `deny_unknown_fields`.** A misspelt key is a load error naming the key, not
a silent default.

- [Scenario top level](#scenario-top-level)
- [`[sim]` dials](#sim-dials)
- [`[terrain]`](#terrain)
- [Placements](#placements)
- [Doctrine and orders](#doctrine-and-orders)
- [Library: units.toml](#library-unitstoml)
- [Library: weapons.toml](#library-weaponstoml)
- [Library: sensors.toml](#library-sensorstoml)
- [Library: air.toml](#library-airtoml)
- [Library: air_defence.toml](#library-air_defencetoml)
- [Library: c2.toml](#library-c2toml)
- [Library: terrain_types.toml](#library-terrain_typestoml)
- [The input contract](#the-input-contract)
- [Metrics](#metrics)
- [Command-line tools](#command-line-tools)
- [Bundled scenarios](#bundled-scenarios)

---

## Scenario top level

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | *required* | The scenario's name. Also how the app's picker and `--only` refer to it |
| `default_seed` | integer | 0 | Seed for terrain **and** the dice. Sweep it to average over maps (§14.1) |
| `[terrain]` | table | *required* | The ground. See below |
| `[sim]` | table | all defaults | Model dials |
| `[blue]`, `[red]` | table | empty | The two forces |

A file is recognised as a scenario by parsing as one: it needs `name` and `[terrain]`, which no
library has. There is no hard-coded list of scenario names.

## `[sim]` dials

Every dial has a default, so a scenario states only what it changes.

| Dial | Type | Default | § | Meaning |
|---|---|---|---|---|
| `dt_s` | float > 0 | 1.0 | §7.1 | Integration tick length, seconds - the continuous cadence |
| `epoch_s` | float > 0 | 10.0 | §7.1 | Decision-epoch length, seconds - the discrete cadence |
| `suppression_radius_m` | float ≥ 0 | 35.0 | §4.3 | A round landing this close is a near miss. Indirect fire only |
| `p_suppress` | float in [0,1] | 0.15 | §4.3 | Probability one near miss steps suppression up a level |
| `recover_per_s` | float ≥ 0 | 0.05 | §4.3 | Rate of stepping back down a level |
| `suppressed_fire_factor` | float in [0,1] | 0.4 | §4.3 | Outgoing fire multiplier while Suppressed |
| `track_hold_s` | float ≥ 0 | 45.0 | §10.1 | How long a track survives without re-observation |
| `track_maintain_p` | float in [0,1] | 0.5 | §10.1 | How good a look must be, as P(glimpse this epoch), to refresh a track |
| `allocation` | `optimal` \| `greedy` \| `independent` | `optimal` | §10.2 | How side-wide fire allocation is solved |
| `allocation_horizon` | integer ≥ 1 | 1 | §10.6 | How many epochs the allocation objective prices. 1 is an exact identity |
| `max_batteries_per_air_target` | integer | 2 | §11.2 | Overkill cap: air-defence batteries per airframe. **Air only** - ground fires have no cap |
| `fires_need_c2` | bool | `false` | §11.3 | Must a ground shooter be inside a live friendly post's radius to join the side-wide fire plan? |
| `sensor_tasking` | bool | `false` | §10.3 | Do steerable sensors re-point themselves by expected information gain? |
| `risk_weight` | float ≥ 0 | 50.0 | §10.5 | Default exchange rate: metres of movement cost spent to avoid one unit of exposure. A unit may override it |
| `fire_risk_weight` | float ≥ 0 | 0.0 | §10.5 | How heavily enemy *weapon reach* counts in the movement risk raster, beside enemy observation |
| `repath_margin` | float ≥ 0 | 0.1 | §10.5 | How much better a new route must be, as a fraction, before a planning unit switches |
| `belief_cells` | integer ≥ 1 | 48 | §10.3 | Edge length of the coarse belief **and movement-planning** grid |

**Four are switches back to older behaviour**, which is how one model is isolated from another:
`allocation = "independent"`, a long `track_hold_s`, `sensor_tasking = false` and
`fires_need_c2 = false`.

**Three cannot be changed while the app is running** – `dt_s`, `epoch_s` and `belief_cells` –
because doing so would make the two halves of a trial answer different questions, or require
rasters to be rebuilt and re-keyed. The panel shows them read-only rather than hiding them.

## `[terrain]`

| Key | Type | Meaning |
|---|---|---|
| `cell_size_m` | float | Metres per cell. Default scenarios use 10.0 |
| `width_cells` | integer | Grid width. World width is `width_cells × cell_size_m` |
| `height_cells` | integer | Grid height |
| `source` | table | One of the four forms below |

Positions everywhere are **world metres**, not cells. X is east, Y is north, origin at the
south-west corner.

### `[terrain.source.flat]`

| Key | Type | Meaning |
|---|---|---|
| `elevation_m` | float | A dead-flat plane at this height |

The right choice for a validation fixture: it takes terrain out of the answer entirely.

### `[terrain.source.hills]`

| Key | Type | Meaning |
|---|---|---|
| `count` | integer | Number of Gaussian hills summed to make the relief |
| `max_height_m` | float | Peak height of the tallest |
| `base_radius_m` | float | Characteristic hill width |
| `woods_fraction` | float in [0,1] | Fraction of the map painted Trees |
| `urban_blocks` | integer | Number of rectangular urban blocks |

### `[terrain.source] preset = "..."`

One of `rolling_hills`, `wooded_hills`, `light_urban`, `dense_urban`, `mountain_pass`,
`flat_plain`. Sugar for a recipe, so a preset can always be copied out and adjusted.

### `[terrain.source.layers]`

A recipe: a base surface plus ordered feature layers.

| Key | Type | Meaning |
|---|---|---|
| `base` | table | `{ flat = {...} }` or `{ hills = {...} }`, as above |
| `apply` | array of tables | Feature layers, applied **in the order written** |

The key is **`apply`**, not `features`. Layer entries:

| Layer | Keys |
|---|---|
| `ridge` | `bearing_deg`, `crest_m`, `width_m` |
| `woodland` | `fraction`, `patch_scale_m` |
| `urban` | `blocks`, `min_size_m`, `max_size_m` |

Urban after woodland leaves urban; the reverse does not. All layers draw from one seeded stream,
so a recipe plus a seed always reproduces the same map (V53).

## Placements

Each force table (`[blue]` / `[red]`) may hold these arrays. Every placement needs an `id`
unique across the whole scenario, a `type` from the matching library, and a `pos`.

### `[[side.units]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | *required* | Unique id |
| `type` | string | *required* | Key into `units.toml` |
| `pos` | `[x, y]` | *required* | World metres |
| `route` | array of `[x, y]` | none | Scripted waypoints, followed exactly |
| `objective` | `[x, y]` | none | Plan its own way here, re-solved each epoch |
| `risk_weight` | float | `[sim] risk_weight` | This unit's caution |

**`route` and `objective` are mutually exclusive**: declaring both is a load error naming the
unit (V72). The same exclusivity holds when they are set from the app.

### `[[side.sensors]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | *required* | Unique id |
| `type` | string | *required* | Key into `sensors.toml` |
| `pos` | `[x, y]` | *required* | World metres |
| `facing_deg` | float | 0.0 | Arc centre. 0 is east, increasing anticlockwise. Only meaningful with a finite `for_width_deg` |

### `[[side.jammers]]`

| Key | Type | Meaning |
|---|---|---|
| `pos` | `[x, y]` | World metres |
| `power` | float in [0,1] | Peak degradation at the centre; 0 at the edge |
| `radius_m` | float ≥ 0 | Effective radius |

A jammer has no id and cannot be targeted. It degrades the **enemy's** detection of its own side
(§8.1) and shrinks the **enemy's** C2 (Command and Control) coordination radius (§11.1): the same
asset, both signs.

### `[[side.air]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | *required* | Unique id |
| `type` | string | *required* | Key into `air.toml` |
| `pos` | `[x, y]` | *required* | Start position |
| `altitude_m` | float ≥ 0 | 0.0 | Altitude in the chosen frame |
| `altitude_ref` | `agl` \| `amsl` | `agl` | **Decides whether terrain can mask it** (§9.1) |
| `heading_deg` | float | 0.0 | Initial heading |
| `speed_m_s` | float | type's `cruise_speed_m_s` | Per-instance override |
| `waypoints` | array of `[x, y]` | empty | The flight path |
| `terminal` | `"hold"` \| `{ orbit = {...} }` | `hold` | What to do at the last waypoint |
| `target` | `{ unit = "id" }` or `{ point = [x,y] }` | none | Aim point for a strike payload |

`terminal = { orbit = { radius_m = 800.0, clockwise = true } }` orbits the final waypoint.

**`target = { unit = "..." }` resolves across units, air-defence batteries and C2 posts**: one
namespace, so aiming at a SAM (Surface-to-Air Missile) needs no new syntax. `asset` is a clearer
alias for the same key.

### `[[side.air_defence]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | *required* | Unique id |
| `type` | string | *required* | Key into `air_defence.toml` |
| `pos` | `[x, y]` | *required* | World metres |
| `self_cue` | bool | `true` | May the battery act on its **own** radar's track? False forces it onto the net and it pays `cue_latency_s` |
| `emitting` | bool | `true` | Is the radar **transmitting**? False is EMCON: it detects nothing, cues nothing, contributes no coverage, and gives an ARM nothing to home on |

**These are two different decisions** (§12.5). `self_cue` is *whose track it acts on*; `emitting`
is *whether the radar runs at all*.

### `[[side.c2]]`

| Key | Type | Meaning |
|---|---|---|
| `id` | string | Unique id |
| `type` | string | Key into `c2.toml` |
| `pos` | `[x, y]` | World metres |

## Doctrine and orders

### `[side.doctrine]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `priority` | array of strings | `["all"]` | Tiers, highest first |
| `mode` | `strict` \| `weighted` | `strict` | Whether a tier *decides* or merely *biases* |

A priority entry may name an **id**, a **role**, a **class** (`unit`, `air_defence`, `c2`,
`air`) or **`"all"`**, checked in that order. The **first** matching entry decides the tier. A
role never masks its class. **A name matching nothing is a load error** listing what would have
worked.

There is no "no doctrine": omitting the block gives `priority = ["all"]`, which *is* the
undirected behaviour, so the identity holds by construction (§13.1).

### `[[side.orders]]`

| Key | Type | Meaning |
|---|---|---|
| `shooter` | string | Asset id that must engage |
| `target` | string | Asset id to engage |

An ordered shooter is removed from the assignment problem entirely. The order lapses while the
target is dead or unreachable, and resumes when it reappears.

---

## Library: `units.toml`

One table per unit type. Model: §3.1, §4.1.

| Field | Type | Meaning |
|---|---|---|
| `height_m` | float | Actor height for line of sight and slant range |
| `silhouette_width_m` | float | Target width $W$ for the direct-fire hit integral |
| `element_count` | integer | Sub-elements. Drives fire **volume** and how much there is to kill |
| `speed_m_s` | float | Cross-country pace. 0.0 means emplaced |
| `weapon` | string, optional | Key into `weapons.toml`. Absent means unarmed |
| `[type.signature]` | table | Per-modality detectability in [0,1], e.g. `optical = 0.75` |
| `value` | float, optional | What killing this is worth. Omitted means derived from size and threat |
| `role` | string, optional | Free-form label a fire plan can sort on |

## Library: `weapons.toml`

One table per weapon. Model: §2.1-§2.3, §12.3.

| Field | Type | Class | Meaning |
|---|---|---|---|
| `class` | `direct` \| `indirect` | both | **Decides which model resolves the shot** |
| `rof_rounds_per_min` | float | both | Rate of fire, per element |
| `max_range_m` | float | both | Maximum engagement range, slant |
| `min_range_m` | float | indirect | Inner dead zone. Easy to forget and expensive to forget |
| `dispersion_mrad` | float | direct | 1σ angular error; $\sigma(r) = \text{mrad} \cdot r/1000$ m |
| `p_kill_given_hit` | float in [0,1] | direct | Lethality once a round strikes the silhouette |
| `moving_target_penalty` | float | direct | Factor inflating σ against a moving target |
| `cep_m` | float > 0 | indirect | Circular error probable; $\sigma = \text{CEP}/\sqrt{2\ln 2}$ |
| `lethal_radius_m` | float > 0 | indirect | Carleton kernel scale $R_L$ |
| `engages_air` | bool | both | Opt-in seam for a dual-role gun. Default false; changes nothing today |
| `anti_radiation` | bool | indirect | Homes on a transmitting radar |
| `silent_cep_m` | float, optional | indirect | CEP against a **silent** radar. Absent falls back to `cep_m` |

**Direct fire needs line of sight and no track; indirect fire needs a track and no line of
sight.** That asymmetry is the whole difference between the classes at the decision layer (V70).

## Library: `sensors.toml`

One table per sensor type. Model: §3.1, §3.2.

| Field | Type | Meaning |
|---|---|---|
| `modality` | `optical` | Propagation channel. A tag, so acoustic and EO/IR are additive |
| `mount_height_m` | float | Actor height. **Ignored when the sensor is carried by a drone** |
| `max_range_m` | float | Hard cutoff - beyond it the rate is exactly zero |
| `lambda0_per_s` | float | Peak glimpse rate $\lambda_0$ against a perfect target at zero range |
| `range_half_m` | float > 0 | $r_{1/2}$, the range at which the falloff has halved the rate |
| `range_exponent` | float | $n$, how sharply the falloff bites |
| `for_width_deg` | float, optional | Field of regard. **Omit for all-round** |

**`for_width_deg` is what makes a sensor taskable.** An all-round sensor has nothing to point.

## Library: `air.toml`

One table per airframe type. Model: §9.2, §9.3.

| Field | Type | Meaning |
|---|---|---|
| `height_m` | float | Target height when it is being shot at |
| `silhouette_width_m` | float | Target width |
| `cruise_speed_m_s` | float | Default speed; an instance may override |
| `max_turn_rate_deg_s` | float | **Rate limit on heading**, implying $r_{\min} = v/\omega$ |
| `endurance_s` | float | Time aloft before removal. 0 means unlimited |
| `[type.signature]` | table | Per-modality detectability |
| `sensor` | string, optional | Key into `sensors.toml` - makes it a recce platform |
| `payload` | string, optional | An **indirect** weapon from `weapons.toml` |
| `munitions` | integer | How many releases it carries |
| `expendable` | bool | Does the airframe die with its attack? |
| `release_range_m` | float | Standoff distance at which it releases |
| `autonomous` | bool | May it pick its own target from what its side has located? Default false |
| `value` | float, optional | What killing it is worth. Omitted means derived from munitions and sensor |
| `role` | string, optional | Free-form label |

**`release_range_m` and a defending battery's `max_range_m` are a matched pair.** Getting the
relation wrong is the classic way to build a scenario that measures nothing.

## Library: `air_defence.toml`

One table per battery type. Model: §9.4, §9.5, §12.

| Field | Type | Meaning |
|---|---|---|
| `min_range_m`, `max_range_m` | float | Slant-range envelope |
| `min_alt_m`, `max_alt_m` | float | **Altitude band** - what separates a CIWS from a high-tier SAM |
| `mount_height_m` | float | Height of the launcher and its radar |
| `requires_los` | bool | Does the engagement need a sightline? |
| `reaction_time_s` | float | Crew and system delay once a track is actionable |
| `cue_latency_s` | float | Comms delay paid on any **externally** cued track |
| `magazine` | integer | Interceptors. 0 means unlimited |
| `channels` | integer | Simultaneous engagements - **what a raid saturates** |
| `sensor` | string, optional | Organic radar. **Omitting it is a modelling statement**: the battery can only fire on tracks handed to it |
| `element_count` | integer | Sub-elements, so it takes area damage like a unit (§12.1) |
| `height_m`, `silhouette_width_m` | float | As a target |
| `value` | float, optional | What killing it is worth. **No cross-class derivation** - declare it |
| `role` | string, optional | Free-form label |

Exactly one engagement model, as a sub-table:

| Sub-table | Fields | Law |
|---|---|---|
| `[type.engagement.gun]` | `kill_rate_per_s` | $\text{TTK} \sim \text{Exp}(\lambda_k)$ |
| `[type.engagement.missile]` | `ssk_p`, `missile_speed_m_s`, `reload_s` | shots $\sim$ Geometric($p$) |

**A gun and a missile fail differently, not just at different rates.** Stacking guns wastes
nothing; stacking missiles wastes rounds. That is why coordination pays for one and not the
other (§11.2).

## Library: `c2.toml`

One table per post type. Model: §11.1.

| Field | Type | Meaning |
|---|---|---|
| `coordination_range_m` | float | **Horizontal**, not slant - a link is a comms relationship, not a sightline |
| `link_latency_s` | float | How long a battery must be inside the radius before it is netted. Default 0 |
| `height_m`, `silhouette_width_m` | float | As a target |
| `[type.signature]` | table | Per-modality detectability. **Deliberately high** - being findable is the point |
| `element_count` | integer | Sub-elements, so it takes area damage |
| `value` | float, optional | What killing it is worth |
| `role` | string, optional | Free-form label |

A post has no weapon and no sensor. Killing it costs **no firepower at all**: only the
coordination.

## Library: `terrain_types.toml`

One table per terrain type: `open`, `trees`, `urban`. Model: §1.3.

| Field | Type | Meaning |
|---|---|---|
| `feature_height_m` | float ≥ 0 | Canopy or building height above ground. The blocking surface is $z + f$ |
| `extinction_per_m` | float ≥ 0 | $\kappa$; attenuation per metre of canopy, $\tau = e^{-\kappa L}$ |
| `cover` | float in [0,1] | Protection against **fires** |
| `concealment` | float in [0,1] | Reduction in **detectability** |
| `mobility_cost` | float ≥ 1 | Movement multiplier. `inf` means impassable |

**Cover and concealment are different things.** A wood conceals well and protects poorly; a
built-up area does the reverse. Trees attenuate softly; urban hard-blocks.

Shipped values:

| Type | `feature_height_m` | `extinction_per_m` | `cover` | `concealment` | `mobility_cost` |
|---|---|---|---|---|---|
| `open` | 0.0 | 0.0 | 0.0 | 0.0 | 1.0 |
| `trees` | 12.0 | 0.08 | 0.3 | 0.6 | 1.8 |
| `urban` | 8.0 | 0.0 | 0.7 | 0.5 | 1.5 |

## The input contract

Beyond `deny_unknown_fields`, a **value** the model cannot run on is refused at load, naming the
dial (§7.6, V67).

| Refused | Why |
|---|---|
| `dt_s` ≤ 0 | The clock never advances, so a run never terminates |
| `epoch_s` ≤ 0 | `time_s / epoch_s` is infinite, and the cast to an integer **saturates**, so the epoch loop is handed the maximum boundary and hangs |
| A probability outside [0,1] | `p_suppress`, `track_maintain_p`, `suppressed_fire_factor` |
| A negative duration, radius or weight | `track_hold_s`, `recover_per_s`, `suppression_radius_m`, `risk_weight`, `fire_risk_weight`, `repath_margin` |
| `belief_cells` = 0 | Sizes a raster |
| `allocation_horizon` = 0 | Multiplies the threat term, so a zero horizon prices every target at its size alone - a different objective, silently |
| `sensors.*.range_half_m` ≤ 0 | Reaches a **divisor** in the §3.2 falloff: a zero gives `NaN`, and `NaN` loses every comparison, so sensing goes silently *inert* |
| An indirect weapon's `lethal_radius_m` ≤ 0 | Reaches a divisor in the §2.3 Carleton kernel, same failure |

**The list is deliberately short.** Most zeros are legitimate statements: a stationary drone, a
battery that engages nothing, a direct weapon's unused lethal radius. Refusing those would be
enforcing taste rather than tractability.

The contract runs at load **and** again inside `Sim::new`, so a library patched in memory – which
is what `sweep` does – is held to the same standard as one read from disk.

## Metrics

The 19 columns every study reports, in order. All are read back from the simulation's own event
logs and final state.

| Column | Meaning |
|---|---|
| `blue_losses` | Blue ground sub-elements destroyed |
| `red_losses` | Red ground sub-elements destroyed |
| `blue_units_killed` | Blue units reduced to zero elements |
| `red_units_killed` | Red units reduced to zero elements |
| `detections` | Detection events, both sides, ground and air |
| `first_detection_s` | Time of first contact either way; the run length if none |
| `fire_events` | Fires resolutions that produced casualties |
| `air_launched` | Airframes placed |
| `air_downed` | Airframes shot down |
| `air_leakers` | Airframes that survived to **release** a munition |
| `munitions_released` | Munitions released |
| `ground_casualties_from_air` | Elements killed by air-delivered bursts |
| `ad_shots` | Air-defence shots taken - the denominator for rounds per kill |
| `ad_rounds_left` | Interceptors remaining, finite magazines only |
| `ad_batteries_killed` | Batteries destroyed |
| `c2_posts_killed` | Posts destroyed |
| `blue_cleared_s` | When Blue lost its last ground element |
| `red_cleared_s` | When Red lost its last ground element |
| `epochs` | Decision epochs resolved |

**`*_cleared_s` reports the run length when a side was never cleared**, so `600` means "not by
600 s", not "at 600 s". Read it with the kill counts beside it.

## Command-line tools

All run as `cargo run -p experiments --release --bin <name> -- <args>`. **Always `--release`**:
a debug Monte Carlo study is about twenty times slower.

**A flag given a missing or unparseable value exits with status 2**, naming the flag, rather
than falling back to its default; otherwise `--seeds abc` would quietly run 200 trials and
`--until 60O` would quietly run 600 s, succeeding while answering a different question. The
value-level twin of the scenario schema's `deny_unknown_fields`.

### `batch`

```
batch [dir] [--seeds N] [--until SECONDS] [--out DIR] [--only NAME] [--quiet]
```

| Flag | Default | Meaning |
|---|---|---|
| `dir` | `scenarios` | Folder of scenarios **and** stat-block libraries |
| `--seeds` | 20 | Trials per scenario; seeds `0..N` |
| `--until` | 600 | Simulated seconds per trial |
| `--out` | `out` | Where the CSVs go |
| `--only` | - | Just this scenario, by bare name |
| `--quiet` | - | No progress line |

### `sweep`

```
sweep <scenario> --param PATH (--values a,b,c | --from X --to Y [--steps N])
                 [--seeds N] [--until S] [--metric NAME] [--set PATH=VALUE]...
                 [--dir DIR] [--out DIR] [--quiet] [--quantiles 50,90,95]
```

| Flag | Default | Meaning |
|---|---|---|
| `--param` | *required* | The dotted path to vary |
| `--values` | - | Explicit arm values, comma-separated |
| `--from` / `--to` / `--steps` | `--steps` 5, minimum 2 | A numeric range instead of `--values` |
| `--seeds` | 200 | Trials per arm; seeds `0..N`, shared by every arm |
| `--until` | 600 | Simulated seconds per trial |
| `--metric` | `red_losses` | Which column the paired report is about. Every column still reaches the CSV |
| `--set PATH=VALUE` | - | Repeatable. Pins a dial across **every** arm |
| `--quantiles` | - | Also report these percentiles of the metric, e.g. `50,90,95`. Means only get confidence intervals; quantiles do not |
| `--dir` | `scenarios` | Where to load the scenario and libraries from |
| `--out` | `out` | Where the CSVs go |
| `--quiet` | - | No progress line |

`--param` is a dotted path patched into the TOML **before parsing**, so any field is sweepable.
**The first segment decides which file** is patched: `sensors`, `units`, `weapons`, `air`,
`air_defence`, `c2` and `terrain_types` name libraries; anything else is a scenario path.
Numeric segments index arrays (`red.air.0.altitude_m`). `--values` splits on commas *outside*
brackets, so TOML array literals such as `["c2","all"]` work as a single arm value.

### `factorial`

```
factorial <scenario> --factor PATH=v1,v2 [--factor PATH=v1,v2]...
                     [--seeds N] [--until S] [--metric NAME] [--set PATH=VALUE]...
                     [--dir DIR] [--out DIR] [--quiet]
```

| Flag | Default | Meaning |
|---|---|---|
| `--factor` | *required* | Repeatable. `PATH=v1,v2[,v3...]` - one factor and its levels |
| `--seeds` | 200 | Trials per cell; every cell runs the same seed set |
| `--until` | 600 | Simulated seconds per trial |
| `--metric` | `red_losses` | Which column the report is about |
| `--set PATH=VALUE` | - | Repeatable. Pins a dial across every cell |

Cost is the product of the level counts. Reports main effects and two-way interactions.

### `sensitivity`

```
sensitivity <study.toml> [--seeds N] [--until S] [--dir DIR]
```

| Flag | Default | Meaning |
|---|---|---|
| `--seeds` | 30 | Trials averaged **per design point**, not per arm |
| `--until` | 600 | Simulated seconds per trial |
| `--dir` | `scenarios` | Where to load the scenario and libraries from |

The dial space is a file. See [`studies/README.md`](../studies/README.md). `--seeds` is
deliberately modest: a design point is an average over seeds, and the variance being decomposed
is the one across the *dial space*, not across the dice.

### `meta`

```
meta <scenario> [--metric NAME] [--trajectories N] [--seeds N] [--until S]
                [--levels N] [--only PREFIX,...] [--emit FILE]
```

| Flag | Default | Meaning |
|---|---|---|
| `--metric` | `red_cleared_s` | Which column to screen against |
| `--trajectories` | 12 | Morris trajectories. Cost is `(k+1) x trajectories` |
| `--seeds` | 8 | Trials averaged per design point |
| `--until` | 300 | Simulated seconds per trial |
| `--levels` | 4 | Morris grid levels |
| `--only` | - | Restrict to dials whose path starts with one of these |
| `--emit FILE` | - | Write the screened dial space as a `studies/*.toml` |

Screens **every** dial the loaded libraries expose by Morris elementary effects.

### `findings`

```
findings [--only ID] [--quick]
```

Re-runs every claim in `findings.toml`. Exits non-zero on drift. `--quick` cuts seeds to a
tenth: enough to catch a *broken* finding, far too few to judge drift.

### Probes and benchmarks

```
duel_probe        a direct-fire duel, pair by pair
sensor_siting     what a sensor position is worth
interdiction      the sensing-against-routing game equilibrium
air_raid          a drone raid against air defence
bench             LOS, viewshed, slant range, tick cost, memo hit rate
fires_bench       the fires path alone
```

### The app and the toolchain

```
cargo run -p app                                          # the default scenario
cargo run -p app -- <name|path>                           # a named scenario
FIRES_SIM_SCREENSHOT=out/shot.png cargo run -p app -- <n>  # capture one frame and exit

cargo test --workspace                                    # everything
cargo run -p validation --release --bin validation_report  # the gate table
cargo run -p validation --bin validation_report -- --markdown   # regenerate VALIDATION.md's table
cargo clippy --workspace                                  # lint
cargo fmt                                                 # format
cargo build -p app --release                              # remove dynamic_linking first
```

## Bundled scenarios

| File | Demonstrates | Key dials |
|---|---|---|
| `default.toml` | Terrain generation, mixed forces, sensor placements | - |
| `flat_range.toml` | A featureless test range - isolates a model from terrain | - |
| `mountain_pass.toml` | A composable terrain recipe: base, ridge, woodland, urban | `terrain.source.layers` |
| `fire_allocation.toml` | Four shooters reaching all four targets - where allocation matters | `sim.allocation` |
| `sensor_search.toml` | Narrow-arc observers searching by belief | `sim.sensor_tasking` |
| `covered_approach.toml` | What a covered approach is worth as caution rises | `sim.risk_weight` |
| `kill_chain.toml` | Directed targeting and ground counter-battery | `blue.doctrine.mode` |
| `air_raid.toml` | Counter-air: self-cued against net-cued defences | `air_defence.*.cue_latency_s` |
| `hunter_uas.toml` | What a drone may decide for itself | `air.*.autonomous` |
| `ad_c2.toml` | Coordinated against decentralised air defence | `sim.max_batteries_per_air_target` |
| `fires_c2.toml` | Ground fires and the net | `sim.fires_need_c2` |
| `ew_c2.toml` | Jamming the command link - the soft kill | jammer `power` |
| `sead_arm.toml` | Anti-radiation homing and the go-silent counter | `emitting`, `silent_cep_m` |

Libraries: `units.toml`, `weapons.toml`, `sensors.toml`, `terrain_types.toml`, `air.toml`,
`air_defence.toml`, `c2.toml`. **The last three are optional**: a scenario directory without
them loads with those libraries empty, so an older scenario set still works.

---

*Back to [README](../README.md) · [Guide](GUIDE.md) · [Model](MODEL.md) · [Theory](THEORY.md) ·
[Validation](VALIDATION.md)*
