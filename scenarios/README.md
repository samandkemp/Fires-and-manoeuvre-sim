# Scenarios

TOML data files: scenario definitions (forces, terrain, objectives) and the stat
blocks for units, weapons, and sensors. All numbers here are **abstract placeholder
dials**, never real-world performance data — the models are the product, these are
the knobs.

| File | What it holds |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
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

Every dial in `[sim]` has a default, so a scenario only states what it wants to change.
`dt_s` and `epoch_s` set the continuous and discrete cadences; `track_hold_s` and
`track_maintain_p` set how long a track survives without a fresh observation and how good
a look has to be to count as one. Turning `track_hold_s` up towards the run length
recovers the old permanent-detection behaviour, which is a useful thing to be able to
switch off when isolating another model.
