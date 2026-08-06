# Scenarios

TOML data files: scenario definitions (forces, terrain, objectives) and the stat
blocks for units, weapons, and sensors. All numbers here are **abstract placeholder
dials**, never real-world performance data — the models are the product, these are
the knobs.

| File | What it holds |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
| `fire_allocation.toml` | Four shooters that can all reach all four targets — the case where the allocation rule actually matters |
| `sensor_search.toml` | Narrow-arc observers searching by belief (needs `sensor_tasking`) |
| `air_raid.toml` | The counter-air scenario: a drone raid vs self-cued and net-cued defences |
| `mountain_pass.toml` | A composable terrain recipe: rolling base + ridge + woodland + urban |
| `flat_range.toml` | A flat, featureless test range — isolates models from terrain effects |
| `terrain_types.toml` | Per-terrain-type cover, concealment, mobility, LOS-blocking |
| `units.toml` | Unit stat blocks: elements, speed, signature, suppression response |
| `weapons.toml` | Direct/indirect weapons: range, rate of fire, dispersion, effect |
| `sensors.toml` | Sensors: range/fidelity profile, field of regard, glimpse rate |
| `air.toml` | Drones: altitude, speed, turn rate, endurance, sensor/strike payload |
| `air_defence.toml` | Air defence: gun vs missile engagement, envelope, magazine, cue latency |

`air.toml` and `air_defence.toml` are optional — a scenario directory without them loads
with those libraries empty, so a pre-Phase-9 set still works.

The schema these parse into lives in `crates/sim_core/src/scenario.rs`; the conventions
they follow are in `docs/DESIGN.md`. For a walkthrough of what each dial actually does to
a detection or a round — with worked numbers — see `docs/HOW_IT_WORKS.md`.

## Loading a scenario

```
cargo run -p app                       # the `default` scenario
cargo run -p app -- air_raid           # by bare name, resolved in this directory
cargo run -p app -- path/to/mine.toml  # or by path, for one kept elsewhere
```

The app's **scenario** picker (top of the control panel) lists every file here that parses
as a scenario and switches between them without a restart — terrain, forces and all.

Scenario files are told apart from the stat-block libraries by *being parseable as a
scenario*, not by a hard-coded list of names: a scenario needs a `name` and a `[terrain]`
block, which no library file has. Adding a new library never confuses the picker.

## Describing terrain

`[terrain.source]` takes one of four forms. The first two are the originals; the last two
let a map be *described* rather than picked (`docs/DESIGN.md` §1.3):

```toml
[terrain.source.flat]                       # dead flat
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
from the one seeded stream, so a recipe plus a seed always gives the same map. `base` is
`flat` or `hills`; `apply` may be empty. See `mountain_pass.toml` for a worked example.

## What a scenario contains

| Block | Meaning |
|---|---|
| `name`, `default_seed` | Identity, and the seed used unless a run overrides it |
| `[sim]` | Tick and epoch length, the suppression dials, and the track-decay dials (DESIGN §3.3, §4.3, §10.1) |
| `[terrain]` | Grid size and cell size, and a `source` describing how to generate it |
| `[[blue.*]]` / `[[red.*]]` | Placed `units`, `sensors`, `jammers`, `air`, `air_defence` |

Every placed asset names a `type` from the libraries above, so a scenario says *where*
things are and the libraries say *what they are*.

## The `[sim]` dials

Every dial has a default, so a scenario states only what it wants to change.

| Dial | Default | What it does |
|---|---|---|
| `dt_s` | 1.0 | Tick length — the continuous cadence (DESIGN §7.1) |
| `epoch_s` | 10.0 | Decision-epoch length — the discrete cadence |
| `suppression_radius_m` | 35.0 | A round landing this close is a near miss (§4.3) |
| `p_suppress` | 0.15 | Chance one near miss steps suppression up |
| `recover_per_s` | 0.05 | Rate of stepping back down |
| `suppressed_fire_factor` | 0.4 | Outgoing fire multiplier while Suppressed |
| `track_hold_s` | 45.0 | How long a track survives unobserved (§10.1) |
| `track_maintain_p` | 0.5 | How good a look must be to refresh a track |
| `allocation` | `optimal` | `optimal` / `greedy` / `independent` (§10.2) |
| `max_shooters_per_target` | 3 | Overkill cap per target per epoch |
| `sensor_tasking` | `false` | Do steerable sensors search by belief? (§10.3) |
| `belief_cells` | 48 | Edge length of the coarse belief grid |

Three of these are worth knowing as **switches back to older behaviour**, which is how you
isolate one model from another:

- `allocation = "independent"` restores the pre-Phase-10 rule where every shooter picked
  the nearest enemy for itself. Comparing against `optimal` is what the
  `allocation_gap` experiment measures.
- `track_hold_s` set towards the run length recovers permanent detection — useful when
  you want to study fires without tracks lapsing underneath you.
- `sensor_tasking` is **off by default**: a `facing_deg` you write in a scenario is taken
  as meant. Turn it on to let sensors search (see `sensor_search.toml`), but note it
  dissolves any scenario whose premise is a *committed* sensor posture — the interdiction
  game being the example that caught this.
