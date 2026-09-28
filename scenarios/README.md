# Scenarios

TOML data files: scenario definitions (forces, terrain, objectives) and the stat blocks for
units, weapons, sensors, air, air defence and C2. All numbers here are **abstract
placeholder dials**, never real-world performance data - the models are the product, these
are the knobs.

**The guide to writing these files is [`docs/GUIDE.md`](../docs/GUIDE.md)** - part 2 covers
how to add a unit, weapon, sensor, drone, battery or post, and how to build a scenario to put
them in. [`docs/REFERENCE.md`](../docs/REFERENCE.md) is the field-by-field lookup.

| File | What it holds |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
| `fire_allocation.toml` | Four shooters that can all reach all four targets - where the allocation rule matters |
| `sensor_search.toml` | Narrow-arc observers searching by belief (needs `sensor_tasking`) |
| `covered_approach.toml` | Movement decisions: what a covered approach is worth as caution rises |
| `kill_chain.toml` | Directed targeting, and ground counter-battery |
| `hunter_uas.toml` | What a drone is allowed to decide for itself (`autonomous`) |
| `ad_c2.toml` | Coordinated vs decentralised air defence |
| `fires_c2.toml` | Ground fires and the net (`fires_need_c2`) - and what the overkill cap does when a side is split |
| `sead_arm.toml` | Anti-radiation homing: what a radar's accuracy costs it, and what going silent buys |
| `ew_c2.toml` | Jamming the command link - the soft kill on the same asset SEAD attacks |
| `c2.toml` | C2 post stat blocks: coordination radius, and how findable the post is |
| `air_raid.toml` | The counter-air scenario: a drone raid vs self-cued and net-cued defences |
| `mountain_pass.toml` | A composable terrain recipe: rolling base + ridge + woodland + urban |
| `flat_range.toml` | A flat, featureless test range - isolates models from terrain effects |
| `terrain_types.toml` | Per-terrain-type cover, concealment, mobility, LOS-blocking |
| `units.toml` | Unit stat blocks: elements, speed, signature, suppression response |
| `weapons.toml` | Direct/indirect weapons: range, rate of fire, dispersion, effect |
| `sensors.toml` | Sensors: range/fidelity profile, field of regard, glimpse rate |
| `air.toml` | Drones: altitude, speed, turn rate, endurance, sensor/strike payload |
| `air_defence.toml` | Air defence: gun vs missile engagement, envelope, magazine, cue latency |

`air.toml`, `air_defence.toml` and `c2.toml` are optional - a scenario directory without
them loads with those libraries empty, so an older scenario set still works.

```
cargo run -p app                       # the `default` scenario
cargo run -p app -- air_raid           # by bare name, resolved in this directory
cargo run -p app -- path/to/mine.toml  # or by path, for one kept elsewhere
```

The schema these parse into lives in
[`crates/sim_core/src/scenario/mod.rs`](../crates/sim_core/src/scenario/mod.rs); the models each
dial feeds are specified in [`docs/THEORY.md`](../docs/THEORY.md).
