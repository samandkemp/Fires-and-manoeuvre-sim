# Scenarios

TOML data files: scenario definitions (forces, terrain, objectives) and the stat
blocks for units, weapons, and sensors. All numbers here are **abstract placeholder
dials**, never real-world performance data — the models are the product, these are
the knobs.

| File | What it holds |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
| `air_raid.toml` | The counter-air scenario: a drone raid vs self-cued and net-cued defences |
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
they follow are in `docs/DESIGN.md`.

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

## What a scenario contains

| Block | Meaning |
|---|---|
| `name`, `default_seed` | Identity, and the seed used unless a run overrides it |
| `[sim]` | Tick and epoch length, plus the suppression dials (DESIGN §3.3, §4.3) |
| `[terrain]` | Grid size and cell size, and a `source` describing how to generate it |
| `[[blue.*]]` / `[[red.*]]` | Placed `units`, `sensors`, `jammers`, `air`, `air_defence` |

Every placed asset names a `type` from the libraries above, so a scenario says *where*
things are and the libraries say *what they are*.
