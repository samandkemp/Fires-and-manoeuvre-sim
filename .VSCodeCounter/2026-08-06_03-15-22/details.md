# Details

Date : 2026-08-06 03:15:22

Directory c:\\Users\\Samuel\\Documents\\Coding\\Fires and Manoeuvres Sim

Total : 91 files,  21431 codes, 3081 comments, 2486 blanks, all 26998 lines

[Summary](results.md) / Details / [Diff Summary](diff.md) / [Diff Details](diff-details.md)

## Files
| filename | language | code | comment | blank | total |
| :--- | :--- | ---: | ---: | ---: | ---: |
| [.cargo/config.toml](/.cargo/config.toml) | TOML | 2 | 1 | 1 | 4 |
| [CLAUDE.md](/CLAUDE.md) | Markdown | 282 | 4 | 29 | 315 |
| [Cargo.lock](/Cargo.lock) | TOML | 6,176 | 2 | 597 | 6,775 |
| [Cargo.toml](/Cargo.toml) | TOML | 12 | 11 | 5 | 28 |
| [README.md](/README.md) | Markdown | 354 | 0 | 113 | 467 |
| [SETUP.md](/SETUP.md) | Markdown | 237 | 0 | 63 | 300 |
| [crates/app/Cargo.toml](/crates/app/Cargo.toml) | TOML | 12 | 7 | 3 | 22 |
| [crates/app/src/input.rs](/crates/app/src/input.rs) | Rust | 196 | 22 | 11 | 229 |
| [crates/app/src/main.rs](/crates/app/src/main.rs) | Rust | 241 | 48 | 22 | 311 |
| [crates/app/src/markers.rs](/crates/app/src/markers.rs) | Rust | 243 | 29 | 19 | 291 |
| [crates/app/src/overlays.rs](/crates/app/src/overlays.rs) | Rust | 213 | 38 | 13 | 264 |
| [crates/app/src/selection.rs](/crates/app/src/selection.rs) | Rust | 112 | 16 | 10 | 138 |
| [crates/app/src/state.rs](/crates/app/src/state.rs) | Rust | 79 | 52 | 18 | 149 |
| [crates/app/src/terrain\_view.rs](/crates/app/src/terrain_view.rs) | Rust | 199 | 44 | 18 | 261 |
| [crates/app/src/ui.rs](/crates/app/src/ui.rs) | Rust | 521 | 49 | 25 | 595 |
| [crates/experiments/Cargo.toml](/crates/experiments/Cargo.toml) | TOML | 11 | 3 | 3 | 17 |
| [crates/experiments/src/bin/air\_raid.rs](/crates/experiments/src/bin/air_raid.rs) | Rust | 220 | 51 | 15 | 286 |
| [crates/experiments/src/bin/allocation\_gap.rs](/crates/experiments/src/bin/allocation_gap.rs) | Rust | 163 | 43 | 16 | 222 |
| [crates/experiments/src/bin/batch.rs](/crates/experiments/src/bin/batch.rs) | Rust | 235 | 36 | 26 | 297 |
| [crates/experiments/src/bin/bench.rs](/crates/experiments/src/bin/bench.rs) | Rust | 139 | 22 | 12 | 173 |
| [crates/experiments/src/bin/c2check.rs](/crates/experiments/src/bin/c2check.rs) | Rust | 25 | 1 | 2 | 28 |
| [crates/experiments/src/bin/duel\_probe.rs](/crates/experiments/src/bin/duel_probe.rs) | Rust | 111 | 11 | 8 | 130 |
| [crates/experiments/src/bin/fires\_bench.rs](/crates/experiments/src/bin/fires_bench.rs) | Rust | 107 | 6 | 6 | 119 |
| [crates/experiments/src/bin/interdiction.rs](/crates/experiments/src/bin/interdiction.rs) | Rust | 165 | 21 | 12 | 198 |
| [crates/experiments/src/bin/pd\_sweep.rs](/crates/experiments/src/bin/pd_sweep.rs) | Rust | 67 | 10 | 9 | 86 |
| [crates/experiments/src/bin/risk\_path.rs](/crates/experiments/src/bin/risk_path.rs) | Rust | 64 | 8 | 6 | 78 |
| [crates/experiments/src/bin/sensor\_siting.rs](/crates/experiments/src/bin/sensor_siting.rs) | Rust | 72 | 9 | 9 | 90 |
| [crates/sim\_core/Cargo.toml](/crates/sim_core/Cargo.toml) | TOML | 15 | 8 | 3 | 26 |
| [crates/sim\_core/src/air.rs](/crates/sim_core/src/air.rs) | Rust | 332 | 139 | 37 | 508 |
| [crates/sim\_core/src/air\_defence.rs](/crates/sim_core/src/air_defence.rs) | Rust | 338 | 131 | 30 | 499 |
| [crates/sim\_core/src/allocation.rs](/crates/sim_core/src/allocation.rs) | Rust | 301 | 95 | 26 | 422 |
| [crates/sim\_core/src/c2.rs](/crates/sim_core/src/c2.rs) | Rust | 52 | 48 | 9 | 109 |
| [crates/sim\_core/src/ew.rs](/crates/sim_core/src/ew.rs) | Rust | 19 | 14 | 4 | 37 |
| [crates/sim\_core/src/fires.rs](/crates/sim_core/src/fires.rs) | Rust | 85 | 37 | 14 | 136 |
| [crates/sim\_core/src/game.rs](/crates/sim_core/src/game.rs) | Rust | 62 | 22 | 11 | 95 |
| [crates/sim\_core/src/lib.rs](/crates/sim_core/src/lib.rs) | Rust | 18 | 15 | 4 | 37 |
| [crates/sim\_core/src/los.rs](/crates/sim_core/src/los.rs) | Rust | 279 | 76 | 26 | 381 |
| [crates/sim\_core/src/movement.rs](/crates/sim_core/src/movement.rs) | Rust | 108 | 18 | 12 | 138 |
| [crates/sim\_core/src/pomdp.rs](/crates/sim_core/src/pomdp.rs) | Rust | 127 | 29 | 12 | 168 |
| [crates/sim\_core/src/scenario.rs](/crates/sim_core/src/scenario.rs) | Rust | 355 | 185 | 44 | 584 |
| [crates/sim\_core/src/sensing.rs](/crates/sim_core/src/sensing.rs) | Rust | 193 | 76 | 19 | 288 |
| [crates/sim\_core/src/sim/commands.rs](/crates/sim_core/src/sim/commands.rs) | Rust | 95 | 55 | 20 | 170 |
| [crates/sim\_core/src/sim/counter\_air.rs](/crates/sim_core/src/sim/counter_air.rs) | Rust | 334 | 79 | 23 | 436 |
| [crates/sim\_core/src/sim/detection.rs](/crates/sim_core/src/sim/detection.rs) | Rust | 249 | 95 | 19 | 363 |
| [crates/sim\_core/src/sim/engagement.rs](/crates/sim_core/src/sim/engagement.rs) | Rust | 288 | 93 | 28 | 409 |
| [crates/sim\_core/src/sim/events.rs](/crates/sim_core/src/sim/events.rs) | Rust | 38 | 37 | 7 | 82 |
| [crates/sim\_core/src/sim/los\_cache.rs](/crates/sim_core/src/sim/los_cache.rs) | Rust | 155 | 59 | 25 | 239 |
| [crates/sim\_core/src/sim/mod.rs](/crates/sim_core/src/sim/mod.rs) | Rust | 294 | 77 | 38 | 409 |
| [crates/sim\_core/src/sim/setup.rs](/crates/sim_core/src/sim/setup.rs) | Rust | 301 | 46 | 15 | 362 |
| [crates/sim\_core/src/sim/state.rs](/crates/sim_core/src/sim/state.rs) | Rust | 64 | 55 | 9 | 128 |
| [crates/sim\_core/src/sim/tasking.rs](/crates/sim_core/src/sim/tasking.rs) | Rust | 268 | 95 | 25 | 388 |
| [crates/sim\_core/src/suppression.rs](/crates/sim_core/src/suppression.rs) | Rust | 35 | 12 | 6 | 53 |
| [crates/sim\_core/src/terrain.rs](/crates/sim_core/src/terrain.rs) | Rust | 628 | 177 | 64 | 869 |
| [crates/validation/Cargo.toml](/crates/validation/Cargo.toml) | TOML | 11 | 8 | 3 | 22 |
| [crates/validation/src/bin/validation\_report.rs](/crates/validation/src/bin/validation_report.rs) | Rust | 82 | 18 | 8 | 108 |
| [crates/validation/src/gates.rs](/crates/validation/src/gates.rs) | Rust | 67 | 18 | 3 | 88 |
| [crates/validation/src/lib.rs](/crates/validation/src/lib.rs) | Rust | 97 | 30 | 13 | 140 |
| [crates/validation/tests/air.rs](/crates/validation/tests/air.rs) | Rust | 183 | 32 | 17 | 232 |
| [crates/validation/tests/allocation.rs](/crates/validation/tests/allocation.rs) | Rust | 220 | 40 | 16 | 276 |
| [crates/validation/tests/c2.rs](/crates/validation/tests/c2.rs) | Rust | 184 | 42 | 12 | 238 |
| [crates/validation/tests/catalogue.rs](/crates/validation/tests/catalogue.rs) | Rust | 92 | 11 | 7 | 110 |
| [crates/validation/tests/counter\_air.rs](/crates/validation/tests/counter_air.rs) | Rust | 346 | 42 | 27 | 415 |
| [crates/validation/tests/decision\_identity.rs](/crates/validation/tests/decision_identity.rs) | Rust | 141 | 24 | 7 | 172 |
| [crates/validation/tests/ew\_pomdp.rs](/crates/validation/tests/ew_pomdp.rs) | Rust | 143 | 23 | 19 | 185 |
| [crates/validation/tests/fires.rs](/crates/validation/tests/fires.rs) | Rust | 97 | 10 | 10 | 117 |
| [crates/validation/tests/game.rs](/crates/validation/tests/game.rs) | Rust | 64 | 13 | 8 | 85 |
| [crates/validation/tests/los.rs](/crates/validation/tests/los.rs) | Rust | 319 | 49 | 29 | 397 |
| [crates/validation/tests/movement.rs](/crates/validation/tests/movement.rs) | Rust | 141 | 17 | 11 | 169 |
| [crates/validation/tests/scenario.rs](/crates/validation/tests/scenario.rs) | Rust | 155 | 8 | 15 | 178 |
| [crates/validation/tests/sensing.rs](/crates/validation/tests/sensing.rs) | Rust | 98 | 14 | 12 | 124 |
| [crates/validation/tests/sim\_loop.rs](/crates/validation/tests/sim_loop.rs) | Rust | 1,107 | 100 | 62 | 1,269 |
| [crates/validation/tests/suppression.rs](/crates/validation/tests/suppression.rs) | Rust | 74 | 10 | 9 | 93 |
| [crates/validation/tests/tasking.rs](/crates/validation/tests/tasking.rs) | Rust | 199 | 25 | 13 | 237 |
| [crates/validation/tests/terrain.rs](/crates/validation/tests/terrain.rs) | Rust | 296 | 37 | 26 | 359 |
| [docs/DESIGN.md](/docs/DESIGN.md) | Markdown | 909 | 0 | 244 | 1,153 |
| [docs/HOW\_IT\_WORKS.md](/docs/HOW_IT_WORKS.md) | Markdown | 458 | 0 | 165 | 623 |
| [scenarios/README.md](/scenarios/README.md) | Markdown | 98 | 0 | 24 | 122 |
| [scenarios/ad\_c2.toml](/scenarios/ad_c2.toml) | TOML | 69 | 24 | 15 | 108 |
| [scenarios/air.toml](/scenarios/air.toml) | TOML | 33 | 15 | 4 | 52 |
| [scenarios/air\_defence.toml](/scenarios/air_defence.toml) | TOML | 40 | 22 | 4 | 66 |
| [scenarios/air\_raid.toml](/scenarios/air_raid.toml) | TOML | 77 | 22 | 17 | 116 |
| [scenarios/c2.toml](/scenarios/c2.toml) | TOML | 12 | 10 | 3 | 25 |
| [scenarios/default.toml](/scenarios/default.toml) | TOML | 71 | 20 | 16 | 107 |
| [scenarios/fire\_allocation.toml](/scenarios/fire_allocation.toml) | TOML | 49 | 19 | 14 | 82 |
| [scenarios/flat\_range.toml](/scenarios/flat_range.toml) | TOML | 8 | 2 | 4 | 14 |
| [scenarios/mountain\_pass.toml](/scenarios/mountain_pass.toml) | TOML | 42 | 9 | 14 | 65 |
| [scenarios/sensor\_search.toml](/scenarios/sensor_search.toml) | TOML | 49 | 20 | 13 | 82 |
| [scenarios/sensors.toml](/scenarios/sensors.toml) | TOML | 36 | 13 | 6 | 55 |
| [scenarios/terrain\_types.toml](/scenarios/terrain_types.toml) | TOML | 18 | 8 | 4 | 30 |
| [scenarios/units.toml](/scenarios/units.toml) | TOML | 24 | 4 | 4 | 32 |
| [scenarios/weapons.toml](/scenarios/weapons.toml) | TOML | 31 | 5 | 7 | 43 |

[Summary](results.md) / Details / [Diff Summary](diff.md) / [Diff Details](diff-details.md)