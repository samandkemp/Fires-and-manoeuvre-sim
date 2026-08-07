# Validation

Every model in this project ships with a **gate**: a test that checks it against a
closed-form result or a stated invariant. There are 66 of them, V1–V66, and they are the
backbone of the whole thing.

## Why a gate is not a regression test

A regression test records what the code did yesterday and complains when that changes. It
is useful, and it is not what is wanted here. It cannot tell you the answer was wrong
yesterday, and it turns every deliberate improvement into a failure you have to bless.

A gate names an **external reference** — an analytical result, a limiting case, an identity
the model must satisfy — and checks against that. So:

- a regression test says *the answer changed*;
- a gate says *the answer is wrong*.

That is the difference the project cares about, and it is why
`cargo run -p validation --release --bin validation_report` prints each gate **beside the
thing it is checked against** rather than as a list of green names. The useful question is
never "are the tests passing" but *is the maths still right, and right against what*.

The corollary is a working rule: **if you change a model and a gate fails, understand why
before you re-baseline it.** That gate is the only thing standing between a model and a
plausible-looking number that is quietly wrong.

## The four kinds of reference

Not every model has a closed form to hit. Where one exists it is used; where it does not,
the gate falls back to the strongest available substitute, and which kind it is matters
when reading the table.

| Kind | What it proves | Example |
|---|---|---|
| **Closed form** | The sampler converges on the analytic answer | V22: Monte Carlo area damage against the Carleton–Gaussian convolution `R²/(σ²+R²)·exp(−d²/2(σ²+R²))` |
| **Independent implementation** | Two unrelated algorithms agree | V27: Dijkstra's path cost against Bellman–Ford; V11: DDA line of sight against a fixed-step oracle |
| **Structural invariant** | A property that must hold whatever the numbers | V7: LOS symmetry; V42: belief stays a normalised distribution; V26: raising `risk_weight` never increases exposure |
| **Identity** | A subsystem switched off leaves the rest bit-identical | V40 (EW), V52 (air), V58 (decisions), V62–V66. See [§7.4](design/07-the-simulation-loop.md) |

**The identity discipline is the one to understand**, because it is what has made eight
phases of additions safe. Every phase added since Phase 8 is *appended* to the loop and
draws **zero** random numbers when its inputs are empty. So a scenario with no aircraft
produces the same event log, byte for byte, that it did before the air model existed —
not approximately, exactly. Adding a subsystem cannot silently perturb an existing result,
and if it did, a gate fails immediately rather than a finding quietly rotting.

## The gates

Grouped by the subsystem they constrain. The `Reference` column is abbreviated;
`validation_report` prints each one in full.

### Terrain and line of sight — [§1](design/01-terrain-and-los.md)

| Gate | Property | Reference |
|---|---|---|
| V1 | World↔cell round-trip | `world_to_cell(cell_center(c)) == c` for all cells |
| V2 | Bilinear exactness | An affine field `z = ax+by+c` samples back exactly |
| V3 | Derived layers well-formed | cover, concealment ∈ [0,1]; mobility ≥ 1; no NaN |
| V4 | Generation determinism | Same seed → bit-identical raster; different seed differs |
| V5 | Flat-plane visibility | Two actors, `h > 0`, flat open ground: clear, `τ = 1` |
| V6 | Single-wall shadow | Hidden zone and mask height match the similar-triangles closed form |
| V7 | LOS symmetry | `los(a,b) == los(b,a)` on random terrain |
| V8 | LOS monotonicity | Raising either endpoint never loses visibility |
| V9 | Rigid-motion invariance | Invariant under whole-scenario translation and 90° rotation |
| V10 | Canopy extinction | A Trees strip of width `w` crossed square-on gives `τ = e^{−κw}` exactly |
| V11 | DDA vs oracle | Agrees with an independent fixed-step sampler to a step-driven tolerance |
| V12 | Flat viewshed | On a flat plane the viewshed is exactly the in-range cell set |
| V13 | Ridge shadow | Per-column shadow matches the V6 closed form |
| V53 | Terrain recipes | Recipe + seed reproduces bit-identically; each layer meets its own invariant; layer order is significant |

### Sensing — [§3](design/03-sensing.md)

| Gate | Property | Reference |
|---|---|---|
| V14 | Detection-time distribution | Monte Carlo mean detection time = `1/λ` within CI |
| V15 | Detection closed form | MC frequency by `t` within binomial CI of `1 − e^{−λt}` |
| V16 | Rate structure | `λ` monotone in range and concealment, linear in signature, 0 when gated |
| V17 | **Tick-size invariance** | Compounded per-tick survival equals `e^{−λt}` for *any* `dt` |
| V18 | Sensing determinism | Same `(scenario, seed)` → identical event log |

V17 is the one that earns its keep. Modelling detection as a *rate* rather than a per-tick
probability is what makes the answer independent of the integrator; a per-tick probability
would make the physics a function of `dt`, which is a very hard bug to see and a very easy
one to ship.

### Fires — [§2](design/02-fires.md)

| Gate | Property | Reference |
|---|---|---|
| V19 | Direct-fire hit probability | MC impacts inside `W×H` within CI of the erf product |
| V20 | Hit monotonicity | Falls with range and cover, rises with target size; 0 when blocked |
| V21 | Indirect CEP | Empirical median miss distance = `cep_m` within CI (Rayleigh) |
| V22 | Area-damage closed form | MC Carleton damage against the Gaussian convolution |
| V23 | Damage monotonicity | Falls with offset and cover, rises with lethal radius |
| V24 | Fires determinism | Same `(scenario, seed, mission)` → identical rounds and strengths |

### Suppression and attrition — [§4](design/04-suppression-and-attrition.md)

| Gate | Property | Reference |
|---|---|---|
| V28 | Stationary distribution | Birth–death occupancy `π_k ∝ (β/μ)^k` |
| V29 | Recovery time | Mean time Pinned→Free = `2/recover_per_s` (two exponential steps) |
| V30 | **Lanchester square law** | An aimed-fire duel conserves `A² − B²` in the mean |
| V31 | Suppression gates fire | Pinned emits nothing; Suppressed output = factor × Free |

V30 is the strongest single check in the suite, because the square law is an *emergent*
property of the whole loop — element counts, rate of fire, hit probability, removal — and
not of any one function. Nothing was written to produce it.

### Movement — [§5](design/05-movement-as-dp.md)

| Gate | Property | Reference |
|---|---|---|
| V25 | Zero risk = shortest path | Closed-form 8-connected distance |
| V26 | Risk avoidance monotone | Raising `risk_weight` never increases exposure along the optimum |
| V27 | Path optimality | Dijkstra cost matches an independent Bellman–Ford reference |

### Game theory and movement in-loop — [§6](design/06-game-theory.md)

| Gate | Property | Reference |
|---|---|---|
| V32 | Matching pennies | Value → 0, both strategies → (½, ½) |
| V33 | Rock–paper–scissors | Value → 0, both strategies → uniform |
| V34 | Saddle point | A game with a pure equilibrium converges to that value |
| V35 | Strict dominance | A strictly dominated strategy converges to ~0 weight |
| V36 | Skew-symmetric fairness | `A = −Aᵀ` ⟹ value 0; the value bracket closes |
| V37 | Route following | A unit on a straight route is at `speed·t` after `t` seconds |
| V38 | Pinned unit halts | A Pinned unit does not advance |
| V39 | Interdiction sanity | An unwatched route is safe, so Red weights it and the value falls |

### EW and partial observability — [§8](design/08-ew-and-partial-observability.md)

| Gate | Property | Reference |
|---|---|---|
| V40 | EW modifier | No jammers ⟹ factor exactly 1 (**identity**); jamming cuts detection monotonically |
| V41 | Tiger problem | Exact Bayes posteriors: 0.85 after one observation, 0.9698 after two |
| V42 | Belief well-formed | Stays a normalised distribution; a peaked likelihood lowers entropy |
| V43 | Negative information | Repeated non-detection shifts belief into dead ground |

### Air and counter-air — [§9](design/09-air-and-counter-air.md)

| Gate | Property | Reference |
|---|---|---|
| V44 | Altitude and masking | An AMSL drone below a crest is masked where the same drone at AGL is not |
| V45 | Slant range | `√(horizontal² + Δz²)` exactly; reduces to horizontal when `Δz = 0` |
| V46 | Orbit kinematics | Radius holds to ε; a lap closes in `2πR/v` |
| V47 | Transit and turn rate | Straight leg = `speed·t`; a turn's chord = `2R sin(φ/2)` at `R = v/ω` |
| V48 | Gun time-to-kill | `TTK ~ Exp(λ)`: mean `1/λ`, `P(kill by t) = 1 − e^{−λt}` |
| V49 | Missile time-to-kill | Shots ~ Geometric(p); `E[TTK] = t_f/p + (1/p − 1)·t_r` |
| V50 | Cue latency and leakage | Leakage = `exp(−λW_eff)`; critical latency `L* = W + D − R` |
| V51 | Envelope and magazine gating | Exactly zero engagements outside band/LOS/cue/magazine |
| V52 | Air-off **identity** | Empty air phases draw no randomness — the log is bit-identical |

### The decision layer — [§10](design/10-the-decision-layer.md)

| Gate | Property | Reference |
|---|---|---|
| V54 | Removal preserves history | Removal tombstones rather than shifting, so logged indices stay valid |
| V55 | Track lifecycle and EW | A track lapses `track_hold_s` after its last look — so jamming can **break** one, which permanent detection made impossible |
| V56 | Fire allocation | Hungarian matches an exhaustive optimum for `n,m ≤ 6`, is never below greedy, and never picks a forbidden pairing |
| V57 | Belief-driven tasking | A tasked sensor detects where a fixed stare never does, with a shorter mean time-to-detect |
| V58 | Decision-layer **identity** | A scenario with no allocation choice and no taskable sensor reproduces the pre-Phase-10 log bit-identically |
| V61 | Carried-sensor coverage | A recce drone that finds nothing drains its side's belief out of that ground |

### Command and control — [§11](design/11-command-and-control.md)

| Gate | Property | Reference |
|---|---|---|
| V59 | C2-coordinated air defence | A post makes batteries cover one drone each where nearest-first sends them all at one; a dead post costs no battery, only the coordination |
| V62 | The link degrades, not only dies | An enemy jammer scales the post's radius, decohering the defence with nothing destroyed; a zero-power jammer is an exact **identity** |
| V63 | Fires can be made to need C2 | With `fires_need_c2` on, guns under a post coordinate and guns outside do not; with it off, the fire log is bit-identical |

### SEAD — [§12](design/12-sead.md)

| Gate | Property | Reference |
|---|---|---|
| V60 | Air defence is attritable | A strike drone kills a named post; a destroyed battery's organic radar stops emitting |
| V64 | Anti-radiation homing | The same missile lands with `cep_m` against a transmitting radar and `silent_cep_m` against a silent one, the mean miss scaling as the ratio (`E\|miss\| = σ√(π/2)`) |
| V65 | Ground counter-battery | Artillery kills an emitting battery; a silent one cannot be found by indirect fire, though direct fire needs no track |

### The kill chain — [§13](design/13-the-kill-chain.md)

| Gate | Property | Reference |
|---|---|---|
| V66 | Directed targeting | Strict doctrine is *followed*, not weighed — a gun takes a 3% shot at a priority SAM over a 46% shot at a tank; weighted mode does not overturn it; a priority naming nothing is a load error; LOS and range **block** a pairing so a masked priority falls through; and a shooter holds its target until it is dead or unengageable, the held lock still consuming a slot |

## Running them

```
cargo test -p validation                                     # the gates
cargo test --workspace                                       # gates + harness + app tests
cargo run -p validation --release --bin validation_report     # the table, with references
```

`validation_report` is the one to reach for. It prints every gate beside the closed form
it is checked against — which is the artefact worth showing someone who asks whether the
model is any good.

## Where they live, and why there

The gates are a **separate crate** (`crates/validation`), not tests inside `sim_core`.
Two reasons:

1. **They reach the engine through the public API only.** A model that cannot be validated
   from outside has the wrong interface, and putting the gates in a crate that *cannot*
   see private state makes that a structural fact rather than an intention.
2. **The maths reads without its tests.** `sim_core` is meant to be read as a statement of
   the models; 128 test functions interleaved with it would bury that.

There is exactly one exception. V52's zero-draw half asserts a property of the **RNG
stream** — that empty air phases consume no random numbers — which is genuinely internal,
so it stays a unit test inside `sim_core`.

## Adding one

1. Write the model, and with it the reference: the closed form, the independent
   implementation, the invariant, or the identity it must satisfy. If you cannot name one,
   that is worth pausing over — it usually means the model is not yet specified.
2. State the gate in its design section, in the `Validation gates` table at the end.
3. Add the test to `crates/validation/tests/`, named `vNN_what_it_checks`.
4. Add the `Gate { … }` entry to [`crates/validation/src/gates.rs`](../crates/validation/src/gates.rs),
   with the reference written out.

Step 4 is not optional and cannot be forgotten: `tests/catalogue.rs` asserts the
correspondence **in both directions** — every gate in the catalogue names a test that
exists, and every `vNN_*` test in the suite appears in the catalogue. So the printed table
cannot drift out of step with the suite.

## What is deliberately not gated

Determinism is checked *within* a build, not across platforms. Cross-platform
bit-equality would mean giving up the standard library's float functions, and the value is
not worth the cost for a single-user research tool. Float comparisons use explicit
tolerances, chosen per gate and stated in it.

Performance is measured (`bench`, `fires_bench`) but not gated. A timing assertion on a
laptop is a flaky test, not a guarantee — so optimisations are instead pinned by
**bit-identity**: the LOS memo and the parallel rasters are checked to produce exactly what
the serial path produced, which is a stronger claim than "still fast" and does not depend
on the machine.
