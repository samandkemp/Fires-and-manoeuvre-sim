# Scenarios

TOML data files: scenario definitions (forces, terrain, objectives) and the stat
blocks for units, weapons, and sensors. All numbers here are **abstract placeholder
dials**, never real-world performance data — the models are the product, these are
the knobs.

| File | What it holds |
|---|---|
| `default.toml` | The main scenario: terrain generation, forces, sensor placements |
| `flat_range.toml` | A flat, featureless test range — isolates models from terrain effects |
| `terrain_types.toml` | Per-terrain-type cover, concealment, mobility, LOS-blocking |
| `units.toml` | Unit stat blocks: elements, speed, signature, suppression response |
| `weapons.toml` | Direct/indirect weapons: range, rate of fire, dispersion, effect |
| `sensors.toml` | Sensors: range/fidelity profile, field of regard, glimpse rate |

The schema these parse into lives in `crates/sim_core/src/scenario.rs`; the conventions
they follow are in `docs/DESIGN.md`.
