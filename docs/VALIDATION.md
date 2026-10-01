# Validation

Every model in this project ships with a **gate**: a test that checks it against a closed-form
result or a stated invariant. There are 77 of them, V1-V77, and they are the backbone of the
whole thing.

This page is the catalogue and the method. The models themselves are specified in
[`docs/THEORY.md`](THEORY.md), where each section states its own gates alongside the equations
they hold, so if you are reading about a model, its gates are on the same page. Come here for
the complete list, for what a gate is *for*, and for how to add one.

- [Why a gate is not a regression test](#why-a-gate-is-not-a-regression-test)
- [The four kinds of reference](#the-four-kinds-of-reference)
- [Running them](#running-them)
- [The catalogue](#the-catalogue)
- [Where they live, and why there](#where-they-live-and-why-there)
- [Adding one](#adding-one)
- [What is deliberately not gated](#what-is-deliberately-not-gated)

---

## Why a gate is not a regression test

A regression test records what the code did yesterday and complains when that changes. It is
useful, and it is not what is wanted here. It cannot say the answer was wrong yesterday, and it
turns every deliberate improvement into a failure that has to be blessed.

A gate names an **external reference** – an analytical result, a limiting case, an identity the
model must satisfy – and checks against that. So:

- a regression test says *the answer changed*;
- a gate says *the answer is wrong*.

That is the difference this project cares about, and it is why
`validation_report` prints each gate **beside the thing it is checked against** rather than as a
list of green names. The useful question is never "are the tests passing" but *is the maths still
right, and right against what*.

The corollary is a working rule: **if a model changes and a gate fails, understand why before
re-baselining it.** That gate is the only thing standing between a model and a plausible-looking
number that is quietly wrong.

## The four kinds of reference

Not every model has a closed form to hit. Where one exists it is used; where it does not, the gate
falls back to the strongest available substitute, and which kind it is matters when reading the
catalogue.

| Kind | What it proves | Example |
|---|---|---|
| **Closed form** | The sampler converges on the analytic answer | V22: Monte Carlo area damage against the Carleton-Gaussian convolution $\frac{R_L^2}{\sigma^2+R_L^2}\exp\!\left(\frac{-d^2}{2(\sigma^2+R_L^2)}\right)$ |
| **Independent implementation** | Two unrelated algorithms agree | V27: Dijkstra's path cost against Bellman-Ford; V11: DDA line of sight against a fixed-step oracle |
| **Structural invariant** | A property that must hold whatever the numbers | V7: LOS symmetry; V42: belief stays a normalised distribution; V26: raising `risk_weight` never increases exposure |
| **Identity** | A subsystem switched off leaves the rest bit-identical | V40 (EW), V52 (air), V58 (decisions), V62-V66, V72, V75. See [§7.4](THEORY.md#74-the-identity-discipline) |

**The identity discipline is the one to understand**, because it is what has made nine phases of
additions safe. Every phase since the third is *appended* to the loop and draws **zero** random
numbers when its inputs are empty. So a scenario with no aircraft produces the same event log, byte
for byte, that it did before the air model existed: not approximately, but exactly. Adding a subsystem
cannot silently perturb an existing result, and if it did, a gate fails immediately rather than a
finding quietly rotting.

Where possible the identity is **structural** rather than dial-gated: the new code does not run at
all, rather than running with its effect set to zero. Movement decisions build no planner when no
unit declares an objective; doctrine defaults to a single tier over everything, which *is* the
undirected behaviour. A structural identity cannot be broken by someone changing a default.

## Running them

```
cargo test -p validation                                    # the gates
cargo test --workspace                                      # gates + harness + app tests
cargo run -p validation --release --bin validation_report    # the table, with references
```

`validation_report` is the one to reach for. It runs every gate and prints it beside the closed
form it is checked against, which is the artefact worth showing someone who asks whether the model
is any good.

```
cargo run -p validation --bin validation_report -- --markdown
```

regenerates the catalogue below without running anything.

## The catalogue

Grouped by the [`docs/THEORY.md`](THEORY.md) section each gate constrains.

**This table is generated.** It is emitted from `validation::gates::GATES` by
`validation_report --markdown`, and `crates/validation/tests/catalogue.rs` asserts the
correspondence between that catalogue and the suite **in both directions**: every gate names a
test that exists, and every `vNN_*` test appears in the catalogue. So the published table cannot
claim a gate the tests do not enforce, and cannot omit one they do. Edit
[`crates/validation/src/gates.rs`](../crates/validation/src/gates.rs), never this table.

<!-- Generated:  cargo run -p validation --bin validation_report -- --markdown
     Edit crates/validation/src/gates.rs, not this table. -->

#### §1 - Terrain and line of sight

| Gate | Property | Checked against |
|---|---|---|
| V1 | world<->cell round-trip | world_to_cell(cell_center(c)) == c for all cells |
| V2 | bilinear exactness | sampling an affine field z = ax+by+c returns it exactly |
| V3 | derived layers well-formed | cover, concealment in [0,1]; mobility >= 1; no NaN |
| V4 | generation determinism | same seed -> bit-identical raster; different seed differs |
| V5 | flat plane visibility | two actors with h>0 on flat open ground: clear, tau = 1 |
| V6 | single wall shadow | hidden zone and mask_height match the similar-triangles closed form |
| V7 | LOS symmetry | los(a,b) == los(b,a) in clear and tau, on random terrain |
| V8 | LOS monotonicity | raising either endpoint never loses visibility; tau falls with canopy |
| V9 | rigid-motion invariance | invariant under whole-scenario translation and 90-degree rotation |
| V10 | canopy extinction law | a Trees strip of width w crossed square-on gives tau = exp(-kw) exactly |
| V11 | DDA vs fixed-step oracle | agrees with an independent fixed-step sampler within a step-driven tolerance |
| V12 | flat viewshed is a disc | on a flat plane the viewshed is exactly the in-range cell set |
| V13 | ridge shadow | per-column shadow matches the V6 wall closed form |
| V53 | terrain recipes and presets | recipe+seed reproduces bit-identically; each layer meets its own invariant (woodland fraction, ridge crest lift); layer order is significant; presets differ as their names claim |

#### §2 - Fires

| Gate | Property | Checked against |
|---|---|---|
| V19 | direct-fire hit probability | MC impacts inside the WxH rectangle within CI of the erf product |
| V20 | hit-probability monotonicity | falls with range and cover, rises with target size; 0 when blocked |
| V21 | indirect CEP | empirical median miss distance = cep_m within CI (Rayleigh) |
| V22 | area-damage closed form | MC mean Carleton damage = R^2/(s^2+R^2) exp(-d^2/2(s^2+R^2)) |
| V23 | damage monotonicity | falls with offset and cover, rises with lethal radius |
| V24 | fires determinism | same (scenario, seed, mission) -> identical rounds and strengths |
| V70 | indirect eligibility is a track, not a sightline | a howitzer engages a target masked from it by a ridge, because a shell arcs and what it needs is a track somebody on its side holds; a direct-fire gun in the identical position holds its fire. The converse binds too: jamming the observer drives the glimpse rate below track_maintain_p, the track ages out track_hold_s after its last good look, and the indirect shooter's lock is released with it -- while the target is still alive, so the release is the lapse and not the kill. V66 pins the same two rules for direct fire, where the terms are exactly reversed |

#### §3 - Sensing and detection

| Gate | Property | Checked against |
|---|---|---|
| V14 | detection-time distribution | MC mean detection time = 1/lambda within CI |
| V15 | detection closed form | MC frequency by time t within binomial CI of 1 - e^(-lambda t) |
| V16 | rate structure | lambda monotone in range/concealment, linear in signature, 0 when gated |
| V17 | tick-size invariance | compounded per-tick survival equals e^(-lambda t) for any dt |
| V18 | sensing determinism | same (scenario, seed) -> identical event log; different seed differs |

#### §4 - Suppression and attrition

| Gate | Property | Checked against |
|---|---|---|
| V28 | suppression stationary distribution | birth-death chain occupancy pi_k proportional to (beta/mu)^k |
| V29 | recovery time | mean time Pinned->Free = 2/recover_per_s (two exponential steps) |
| V30 | Lanchester square law | aimed-fire duel conserves A^2 - B^2 in the mean |
| V31 | suppression gates fire | Pinned emits nothing; Suppressed output = factor x Free output |

#### §5 - Movement as dynamic programming

| Gate | Property | Checked against |
|---|---|---|
| V25 | zero-risk = shortest path | closed-form 8-connected distance (max-min) + sqrt(2)*min |
| V26 | risk avoidance monotone | raising risk_weight never increases exposure along the optimum |
| V27 | path optimality | Dijkstra cost matches an independent Bellman-Ford reference |

#### §6 - The game-theoretic layer

| Gate | Property | Checked against |
|---|---|---|
| V32 | matching pennies | fictitious play value -> 0, both strategies -> (1/2, 1/2) |
| V33 | rock-paper-scissors | value -> 0, both strategies -> uniform |
| V34 | saddle point | a game with a pure equilibrium converges to that value |
| V35 | strict dominance | a strictly dominated strategy converges to ~0 weight |
| V36 | skew-symmetric fairness | A = -A^T implies value 0; the value bracket closes |
| V37 | route following | a unit on a straight route is at speed*t after t seconds |
| V38 | pinned unit halts | a Pinned unit does not advance along its route |
| V39 | interdiction sanity | an unwatched route is safe, so Red weights it and the value falls |

#### §7 - The simulation loop

| Gate | Property | Checked against |
|---|---|---|
| V67 | the input contract | a dial the model cannot run on is refused at load, naming it: dt_s or epoch_s at zero or negative (both of which fail to *terminate* - a zero epoch makes time/epoch infinite, and the cast to u64 saturates rather than wrapping, so the epoch loop is handed u64::MAX boundaries), probabilities outside [0,1], negative durations and radii, belief_cells = 0, allocation_horizon = 0 (which would price every target at its size alone - a different objective, arrived at silently), a negative risk_weight, fire_risk_weight or repath_margin, and any stat-block dial that reaches a divisor (a sensor's range_half_m, an indirect weapon's lethal_radius_m) where a zero yields NaN and NaN loses every comparison it is in; legitimate zeros (a stationary drone, a direct weapon's unused lethal radius, a probability of exactly 0 or 1) still load, and every shipped scenario and library satisfies the contract |

#### §8 - Electronic warfare and partial observability

| Gate | Property | Checked against |
|---|---|---|
| V40 | EW modifier | no jammers => factor exactly 1 (EW-off is the identity); jamming cuts detection monotonically |
| V41 | Tiger problem | exact Bayes posteriors: 0.85 after one observation, 0.9698 after two |
| V42 | belief well-formed | stays a normalised distribution; a peaked likelihood lowers entropy |
| V43 | negative information | repeated non-detection shifts belief into dead ground; motion raises entropy |

#### §9 - Air: drones and counter-air

| Gate | Property | Checked against |
|---|---|---|
| V44 | altitude and masking | an AMSL drone below a crest is masked where the same drone at AGL is not |
| V45 | slant range | sqrt(horizontal^2 + dz^2) exactly; reduces to horizontal when dz = 0 |
| V46 | orbit kinematics | radius holds to epsilon; a lap closes in 2*pi*R/v |
| V47 | transit and turn rate | straight leg = speed*t; a turn's chord = 2R sin(phi/2) at R = v/omega |
| V48 | gun time-to-kill | TTK ~ Exp(lambda): mean 1/lambda, P(kill by t) = 1 - e^(-lambda t) |
| V49 | missile time-to-kill | shots ~ Geometric(p); E[TTK] = t_f/p + (1/p - 1) t_r |
| V50 | cue latency and leakage | leakage = exp(-lambda W_eff); critical latency L* = W + D - R |
| V51 | envelope and magazine gating | exactly zero engagements outside band/LOS/cue/magazine; channels capped |
| V52 | air-off identity and determinism | empty air phases draw no randomness (log bit-identical); same seed reproduces |

#### §10 - The decision layer

| Gate | Property | Checked against |
|---|---|---|
| V54 | asset removal preserves history | removal tombstones rather than shifting: every index already in an event log still resolves to the same asset |
| V55 | track lifecycle and EW | a track lapses track_hold_s after its last observation and is cleared; continuous observation refreshes it; jamming drives the effective glimpse rate below the maintenance threshold and so breaks a track, which permanent detection made impossible |
| V56 | fire allocation | Hungarian matches an exhaustive optimum for n,m <= 6 and is never below greedy; no slot is reused and no forbidden pairing chosen; a coordinated side spreads fire across targets where the independent rule doubles up; no shooter is committed to two targets in one epoch; allocation draws no randomness |
| V57 | belief-driven sensor tasking | against an enemy hidden outside its initial arc, a belief-tasked sensor detects where a fixed stare never does, with a shorter mean time-to-detect; belief stays a normalised non-negative distribution with finite entropy across many updates (extends V42); tasking draws no randomness, so facings reproduce |
| V58 | decision-layer identity | the Phase 10 phases draw zero randomness: a scenario with no allocation choice and no taskable sensor reproduces the pre-Phase-10 event log bit-identically, and driving the decision phases repeatedly between ticks changes nothing |
| V61 | carried-sensor coverage informs belief | a recce drone that flies over ground and finds nothing drains its side's belief out of that ground, where before carried sensors were excluded from the POMDP layer entirely; belief stays normalised; an emplaced-only scenario is unchanged, because only carried sensors are cache-quantised; the cleared ground moves with the drone, so the raster really is refreshed |
| V72 | movement decisions are a structural identity | a scenario in which no unit declares an `objective` reproduces exactly and its scripted units follow their routes to the metre -- no planner is built and no risk raster computed, because `replan_movement` returns before doing anything. The identity holds by construction rather than by a dial being off, the same argument doctrine's `priority = ["all"]` default makes. Declaring both `route` and `objective` on one unit is refused at load naming the unit, since neither reading of the combination is obviously the one meant. The same exclusivity holds when they are set interactively: a hand-drawn route cancels the objective, and taking an objective clears the route the planner is about to own -- otherwise the planner would overwrite a drawn route at the next epoch and the route would look like it had not taken. A runtime objective is planned and followed, not merely stored |
| V73 | a planned route avoids what is watching it | with an enemy sensor squarely on the straight line to its objective, a unit planning its own route leaves that line by hundreds of metres to pass around the watched ground; with `risk_weight = 0` -- the same map, the same sensor, the same planner -- it stops caring and goes straight. That is V25's zero-risk-is-the-shortest-path arriving inside the loop, and the second half is what makes the first mean something rather than merely proving the router emits some route |
| V74 | a planner does not dither | a watcher on the line makes passing north and passing south cost almost exactly the same, which is the situation that makes a fresh solve wobble between them every epoch. The committed direction must not flip and the unit must arrive. Deliberately NOT monotone progress toward the objective: a detour increases straight-line distance before it decreases it, so that test would forbid routing around anything -- an earlier cut asserted it and failed for the right behaviour. The hysteresis is the movement analogue of SS13.4's target lock |
| V75 | the allocation objective's planning horizon | [sim] allocation_horizon = 1 reproduces the single-epoch objective EXACTLY -- the same fire-event log, event for event, across seeds -- because the horizon multiplies the threat term by 1.0, which is exact in floating point. Above 1 the objective prices what a survivor would go on to do, so a target's threat is weighted against its size and the fire plan genuinely changes. The formula ordering is checked arithmetically: a large harmless target outranks a small dangerous one at horizon 1 and must be overtaken as the horizon grows, monotonically, or the dial would be a uniform rescale changing no decision at all |
| V76 | autonomous strike targeting | an airframe without `autonomous` aims only where it was told or at its flight plan's destination, so every scenario written before this behaves exactly as it did. With the flag, a drone passing within release range of an enemy its own side has located aims at that enemy instead of flying on -- ranked by the SAME value function and doctrine tier the ground shooters use, so a side ordered to kill command posts first does it with drones too. An explicit assignment still wins outright, so opportunity never overrides orders. Checked by where munitions were AIMED, not how many were dropped: a drone with no assignment has always aimed at its destination, so a count cannot tell 'ignored the enemy' from 'attacked it' |
| V77 | fire in the movement risk raster | `[sim] fire_risk_weight = 0` leaves the raster enemy-observation only, exactly as it always was. Above zero, ground within an enemy weapon's reach is priced too, and raising the weight never reduces the clearance taken. The fixture is decisive because it separates the two facts completely: an INDIRECT gun covering a corridor and NO enemy sensor anywhere on the map, so observation risk is identically zero and only the new term can move the route. Direct fire additionally requires a sightline; indirect requires only range, because its eligibility is a track rather than a line |

#### §11 - Command and control

| Gate | Property | Checked against |
|---|---|---|
| V59 | C2-coordinated air defence | with one drone nearest to every battery, nearest-first sends them all at it while a C2 post makes them cover one drone each; a scenario with no post is unchanged from the pre-C2 engine; a dead post coordinates nothing, costing no battery, magazine or envelope - only the coordination; and a post coordinates its OWN side only, so with both sides coordinated each battery still follows its own doctrine and makes the same choice it would make with the enemy's post removed |
| V62 | the C2 link degrades, not only dies | an enemy jammer on the post scales its coordination radius by the EW factor, so the flanking batteries drop out of the net and the defence decoheres with no battery, post, magazine or envelope touched - the soft twin of killing the post; a *friendly* jammer does not cut its own side's net; a zero-power jammer runs the whole arithmetic and changes nothing (§7.4); link_latency_s delays joining, and a battery not yet in the net falls back to nearest-first, so a late link cannot undo engagements already committed |
| V63 | ground fires can be made to need C2 | with [sim] fires_need_c2 on, two guns under a live post engage two targets while the same guns without one both take the nearest - the pre-Phase-10 rule; with the dial off (the default) a C2 post does not touch ground fires at all, so the fire log is bit-identical (§7.4); the net is per shooter, so a post covering two of three guns leaves the third choosing for itself; and jamming scales the post's radius, so a link pulled to 20 m drops a gun 40 m away out of the net while one pulled to 1000 m does not |
| V68 | the overkill discount replaces the overkill cap | with three guns and one target all three engage, where the old hard cap `max_shooters_per_target` assigned nothing to the surplus and they fired nothing at all; with a target each they still take one each, so the geometric discount (1 - qbar)^k delivers the spread the cap was credited with. The cap was removed because it truncated the discount rather than reinforcing it: a shooter with nothing else to engage did nothing, which made a side SPLIT by fires_need_c2 out-fight a coordinated one on fires_c2 (the cap applied once per fire-control problem, so a split side got two) |

#### §12 - SEAD: air defence as a target

| Gate | Property | Checked against |
|---|---|---|
| V60 | SEAD: air defence is attritable | a strike drone assigned a named C2 post destroys it in-simulation and the defence decoheres with no battery lost; a destroyed battery's organic radar stops emitting (sensor_active false); a target id matching nothing yields no aim point, so the new asset lists are additive rather than a replacement |
| V64 | anti-radiation homing | the same missile against the same battery lands with cep_m when its radar is transmitting and silent_cep_m when it is not, the mean miss scaling as the ratio of the two CEPs (E\|miss\| = sigma*sqrt(pi/2), sigma = CEP/sqrt(2 ln 2)) and doing correspondingly less damage; a weapon without the flag ignores the emitter entirely, and an ARM with no silent_cep_m stated falls back to cep_m, so both are exact identities; only a named, live, self-cueing battery counts as an emitter, so an ARM aimed at bare ground is blind by definition |
| V65 | ground counter-battery | a howitzer in range kills an emitting SAM and the fire log names what it hit, which the unit-only target list could not express; a battery that has neither transmitted nor fired is not located, so indirect fire has nothing to aim at - the same counter V64 poses, now costing the defender artillery cover as well as missile accuracy; direct fire needs line of sight and range but no track, so a silent battery in plain view is still a target; a post is located while it is coordinating and stops being so when it has nothing left to coordinate; and a scenario with no enemy emplacements produces exactly the target list it always did (§7.4) |
| V69 | emission and cueing are separate decisions | a battery with emitting = false has its radar OFF: it records zero detections through it, cannot cue itself, and gives an anti-radiation missile nothing to home on; a battery with self_cue = false keeps its radar running and detects exactly as much as one that does not, differing only in whether it may act on its own track or must wait out cue_latency_s. The two were one flag, which let a battery take the missile protection of going dark while still seeing everything -- measured on sead_arm at 1.000 detections while counting as silent. Both defaults are true, so an ordinary battery is unchanged |

#### §13 - The kill chain: directed targeting

| Gate | Property | Checked against |
|---|---|---|
| V66 | the kill chain: directed targeting | a gun with a 46% shot at a near high-value tank and a 3% shot at a far SAM takes the tank unprompted and the SAM when told priority = [air_defence] - strict doctrine is followed, not weighed; the same priority in weighted mode does not overturn the fifteen-fold better shot, which is the difference the two modes exist to express; a directly ordered engagement bypasses the assignment entirely and lapses if its target dies; a side with no doctrine is an exact identity (§7.4); a priority naming nothing on the field is a load error listing what would have worked, and ids, roles and classes are all accepted; air defence follows the same doctrine, taking a further strike drone over a nearer recce one; line of sight and range BLOCK a pairing rather than lowering its score, so a masked priority target does not hold a shooter hostage - both doctrine and an explicit order fall through to what can actually be engaged; and a shooter holds its target until that target is dead or unengageable, with a held lock still counting against that target's discount so two guns with two live targets hold one each |

#### §14 - The measurement machinery

| Gate | Property | Checked against |
|---|---|---|
| V71 | the sensitivity estimator against a closed form | Sobol first-order and total indices from Saltelli sampling match the analytic indices of the Ishigami function f = sin(x1) + a sin^2(x2) + b x3^4 sin(x1) to within Monte-Carlo tolerance. The third input is the one that matters: its first-order index is exactly ZERO -- x3 does nothing alone, so a one-dial sweep of it finds nothing -- while its total index is large, because it acts entirely through x1. That gap is what the estimator exists to expose. Total indices are never below first-order ones, a structural invariant; and Morris screening, an independent estimator, agrees with Sobol about which dials matter. A gate on the measuring instrument rather than on a model, as V27 is for Dijkstra |

77 gates.

### The three worth singling out

**V17, tick-size invariance** (§3). Modelling detection as a *rate* rather than a per-tick
probability is what makes the answer independent of the integrator. A per-tick probability would
make the physics a function of `dt_s`, which is a very hard bug to see and a very easy one to ship.

**V30, Lanchester's square law** (§4). The strongest single check in the suite, because the square
law is an *emergent* property of the whole loop – element counts, rate of fire, hit probability,
removal – and not of any one function. Nothing was written to produce it.

**V67, the input contract** (§7.6). The value-level twin of the schema's `deny_unknown_fields`. A
misspelt *key* takes its default and answers a different question; a *value* outside its domain
does the same thing. The failure mode that motivated it is reachable from an ordinary
`sweep --param sim.epoch_s --from 0`, whose first arm hangs with no diagnostic at all. The list of
refusals is deliberately short: see §7.6 for why refusing every zero would be enforcing taste
rather than tractability.

## Where they live, and why there

The gates are a **separate crate**, `crates/validation`, not tests inside `sim_core`. Two reasons:

1. **They reach the engine through the public API only.** A model that cannot be validated from
   outside has the wrong interface, and putting the gates in a crate that *cannot* see private
   state makes that a structural fact rather than an intention.
2. **The maths reads without its tests.** `sim_core` is meant to be read as a statement of the
   models, and well over a hundred test functions interleaved with it would bury that.

There is exactly one exception. **V52's zero-draw half** asserts a property of the RNG stream -
that empty air phases consume no random numbers, which is genuinely internal, so it stays a unit
test inside `sim_core`.

Suites are grouped by design area – `tests/sead/arm.rs`, `tests/c2/link.rs` – with one
`tests/<group>.rs` per group declaring them by `#[path]`. Cargo builds every `tests/*.rs` as its
own binary linking `sim_core` afresh, so the grouping keeps a new suite from adding another link
unit.

## Adding one

1. **Write the model, and with it the reference**: the closed form, the independent
   implementation, the invariant, or the identity it must satisfy. If none can be named, that is
   worth pausing over: it usually means the model is not yet specified.
2. **State the gate in its `docs/THEORY.md` section**, in that section's validation table.
3. **Add the test** to `crates/validation/tests/`, named `vNN_what_it_checks`. Put a new suite in
   the group its section belongs to; a genuinely new area gets its own.
4. **Add the `Gate { … }` entry** to
   [`crates/validation/src/gates.rs`](../crates/validation/src/gates.rs), with its section and the
   reference written out.
5. **Regenerate the catalogue** above with `validation_report --markdown`.

Step 4 is not optional and cannot be forgotten: `tests/catalogue.rs` asserts the correspondence in
both directions, so a gate with no test, or a V-numbered test not catalogued, is a build failure
rather than a silent gap.

## What is deliberately not gated

**Determinism is checked within a build, not across platforms.** Cross-platform bit-equality would
mean giving up the standard library's float functions, and the value is not worth the cost for a
single-user research tool. Float comparisons use explicit tolerances, chosen per gate and stated in
it.

**Performance is measured but not gated.** A timing assertion on a laptop is a flaky test, not a
guarantee. Optimisations are pinned by **bit-identity** instead: the line-of-sight memo and the
parallel rasters are checked to produce exactly what the serial path produced, which is a stronger
claim than "still fast" and does not depend on the machine.

**Realism is not gated, because it is not testable.** Every parameter in this project is an
abstract placeholder. What can be checked is that the model computes what it says it computes; what
cannot be checked is whether those numbers describe any real system. The gates make the first claim
and deliberately make no attempt at the second.

---

*Back to [README](../README.md) · [Guide](GUIDE.md) · [Model](MODEL.md) · [Reference](REFERENCE.md) · [Theory](THEORY.md)*
