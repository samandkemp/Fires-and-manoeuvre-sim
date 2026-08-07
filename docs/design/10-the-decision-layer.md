[Index](README.md) · [← §9 Air: drones & counter-air](09-air-and-counter-air.md) · [§11 Command and control →](11-command-and-control.md)

---

## 10. The decision layer *(Phase 10 — complete)*

Phases 1–9 modelled everything except anyone *deciding* anything. Fires picked the
nearest enemy, routes were drawn by hand, sensors stared where they were placed, and the
belief filter of §8.2 was computed for a UI overlay that no sim code read. This phase
closes the loop **sensing → belief → decision → action**: tracks now decay (§10.1), fire
is allocated side-wide (§10.2), and sensors point themselves by belief (§10.3).

The decision epoch of §3.3 was designed as the hook for exactly this. It currently calls
only `resolve_fires()`.

Decisions (user, 2026-07-31): tracks decay by **hold time**; fire allocation is
**side-wide and optimal**, with a greedy allocator kept alongside so the optimality gap
is measured rather than assumed; target value is an **optional dial** over a derived
default; sensor tasking maximises **expected information gain**.

### 10.1 Track lifecycle *(landed)*

§3.2 made detection permanent, with track loss deferred to EW. That deferral turned out
to have teeth: a unit once seen stays seen forever, so **jamming a tracked unit does
nothing at all**. EW could prevent a track but never break one, which is half the model
missing rather than a simplification.

Detection is now derived from a last-observation time. `UnitState` and `AirState` carry
`last_seen_s: Option<f64>`, and at each epoch

```
detected  ⟺  now − last_seen_s  <  track_hold_s
```

`detected` stays the field everything else reads, so indirect-fire gating (§4.2) and the
§9.5 cueing timeline needed no change. `track_hold_s` is a scenario dial. Air keeps its
per-sensor `seen_by` record as well, because §9.5 needs to know *which* battery saw the
target.

A lapsed air track clears the whole cueing record — `detected_at_s`, `detected_by` and
`seen_by` — not just the flag. Otherwise reacquisition would find a stale `detected_at_s`
already aged past `cue_latency_s + reaction_time_s` and a battery would fire the instant
the target reappeared, skipping the §9.5 timeline the scenario exists to exercise.

**Maintenance runs at the decision epoch, not the tick.** The glimpse loop skips
already-detected targets, so refreshing a track means looking again — measured at
4 sensors × 6 units × 97 µs ≈ 2.3 ms/tick, up to 20× the whole tick budget. At a 10 s
epoch that amortises to 0.23 ms/tick. The cadence is also right on its own terms: tracks
decay over tens of seconds, and maintaining one is a decision-layer concern.

**Maintenance is deterministic, not a fresh glimpse.** Acquisition stays stochastic;
keeping eyes on something already found is not a coin flip. A track refreshes when the
sensor's *effective* rate `λ_eff` — the §8.1 jammed rate, with concealment, range and
canopy folded in — clears a `track_maintain_p` threshold:

$$
\text{refresh} \iff 1 - e^{-\lambda_{\text{eff}} \Delta t_{\text{epoch}}} \ge p_{\text{maintain}}
$$

Using the effective rate rather than bare geometry is what lets EW break a track:
a jammer that drives `λ_eff` below the threshold ages the track out even with clean LOS.
A pure "can it still be seen?" test would have re-opened the exact gap this closes.
Drawing nothing also leaves the per-tick RNG stream unperturbed.

### 10.2 Fire allocation *(landed)*

Replaces the nearest-enemy rule with a side-wide assignment, solved once per epoch per
side before anyone shoots. For shooter `i` and slot `k` of target `j`,

$$
\text{payoff}\big[i\big]\big[(j,k)\big] = q(i,j)\cdot \text{value}(j)\cdot \big(1 - \bar{q}(j)\big)^{k}
$$

`q(i,j)` is the **fraction of the target destroyed this epoch**, from the existing fires
model — `direct_p_hit` or `expected_area_damage`, times cover, suppression factor and
round count, exactly as a round resolves — clamped to `[0,1]`. Ineligible pairings (out
of range, no LOS, undetected for indirect) are forbidden outright.

`value(j)` is `elements × per_element`, where `per_element` is the optional `value` dial
on the stat block, or `1 + threat/threat_max` when absent, with
`threat = rof × p_kill_given_hit × max_range`. So an unscored stat block still ranks
sensibly — a unit is worth its size, doubled if it is the most dangerous thing on the
field — and doctrine ("kill the radar first") can be stated when wanted. Per *element*,
so a half-destroyed unit is correctly worth less.

`q(i,j)` is an **expectation, clamped** — not a probability. For direct fire it is
`rounds · p_kill / elements`; for indirect, `rounds · E[damage per round]`. Both are linear
in the round count rather than the exact `1 − (1 − q)^rounds`, and both are clamped to
`[0, 1]`. That is fine for *ordering* pairings, which is all the assignment needs, but it
has a consequence worth naming because the same number is reused as `q̄` in the slot
discount below: once `rounds · q` reaches 1 the clamp bites, `(1 − q̄)^k` collapses to zero
for every slot past the first, and the diminishing return becomes a cliff rather than a
curve. Scenarios where one shooter can expect to destroy a whole target in one epoch are
therefore the ones where the discount does least work.

**Slots and the discount.** A target with `E` elements offers `min(E, cap)` slots, and
slot `k` is discounted by `(1 − q̄)^k` for a representative `q̄`: the (k+1)-th shooter
only helps if the `k` before it all failed. This is the standard weapon–target-assignment
decomposition and is exact when the shooters on a target are alike. It turns diminishing
returns into extra columns, keeping the problem a plain linear assignment rather than a
submodular one.

`q̄` is averaged over the shooters that *could* engage the target, not over those actually
assigned to it — which is the only thing available before the problem is solved. The bias
has a direction: a distant shooter that will never be chosen drags `q̄` down, which
under-discounts the later slots and so mildly *encourages* piling on. Exact when the
shooters are alike, as above; worth re-checking with `allocation_gap` on a scenario with
deliberately heterogeneous shooters.

Solved by Hungarian (Kuhn–Munkres) over shooters × slots, with greedy and `independent`
(the old per-shooter rule) alongside. `[sim] allocation` chooses.

**Forbidden pairings are scored zero, not `−∞`.** Kuhn–Munkres produces a *perfect*
matching, while what is wanted is a maximum-weight matching that may leave a shooter
idle. With non-negative payoffs and rows ≤ columns, any partial matching extends to a
perfect one using only zero-weight cells without changing its total — so the two optima
coincide, and assignments landing on a forbidden or worthless cell are simply dropped
afterwards. A large negative sentinel would have been worse than wrong: `1e18 + 10.0`
is `1e18` in `f64`, so every matching with the same number of forbidden cells would have
scored identically.

#### Measured: coordination pays, optimality does not *(2026-08-04)*

`experiments/allocation_gap` runs each scenario under all three rules on the same seeds,
and compares **paired** — the per-seed difference cancels the map and the dice, leaving
only the effect of the rule. On `scenarios/fire_allocation.toml` (four shooters that can
all reach all four targets), 500 seeds:

| Rule | Time to destroy Red | vs `independent`, paired | Targets engaged per epoch |
|---|---|---|---|
| `independent` (the old rule) | 75.1 ± 0.4 s | baseline | 1.00 |
| `greedy` | 63.8 ± 0.4 s | **−11.30 ± 0.47 s** (significant) | 3.02 |
| `optimal` | 63.9 ± 0.4 s | **−11.18 ± 0.49 s** (significant) | 3.02 |

**Coordinating is worth ~15%**, unambiguously, and the mechanism is visible in the spread
column: the old rule sent every gun at the nearest target while three others stood
untouched.

**Optimality is worth nothing measurable.** Greedy and Hungarian differ by 0.12 s against
a standard error of ~0.5, and produce *identical* outcomes on the large majority of seeds
(they diverge at all on well under a fifth). On instances this size — a handful of
shooters against a handful of similar targets — greedy's myopia costs it essentially
nothing, because there is rarely a case where taking the locally best pairing forecloses
a better global one. The optimal solver is kept because it is the reference V56 checks
against and because the gap should be re-measured as scenarios grow, not because it is
currently earning its extra complexity.

> **A methodological correction worth recording.** This section previously claimed greedy
> *beat* the optimal solver "consistently and outside the noise". It did not. That came
> from comparing two unpaired means (30 and 200 seeds) whose difference sat inside the
> sampling error, with no standard error reported to make that visible. A paired test over
> 500 seeds gives a mean difference of −0.12 s with SE 0.22 (t = −0.55): no effect.
> `allocation_gap` now reports a standard error on every figure and pairs every
> comparison, because a bare mean is precisely what invited the wrong conclusion.

On the other shipped scenarios the difference is exactly zero on every seed: with one or
two shooters that can each reach one enemy, all three rules agree. Allocation only matters
when there is a real choice to make, which is why `fire_allocation.toml` had to be built
for this experiment to have anything to measure at all.

### 10.3 Belief-driven sensor tasking *(landed)*

The sim gains a per-side `SpatialBelief` on a coarse grid (`[sim] belief_cells`, default
48), updated each epoch from what the side's sensors *failed* to see and then diffused by
`predict`. This is the point at which the §8.2 POMDP layer stops being a display and
starts driving the simulation.

**The objective is information gain, computed exactly.** For a candidate facing, the
observation is binary per cell: either a sensor detects something at cell `c`, with
probability `b(c)·p(c)`, collapsing the belief to a point mass of zero entropy; or it sees
nothing and the belief becomes `b'(c) ∝ b(c)(1 − p(c))`. So

$$
\mathbb{E}\big[H_{\text{after}}\big] = \left(1 - \sum_c b(c) p(c)\right) H(b')
$$

$$
\text{gain}(\text{facing}) = H(b) - \mathbb{E}\big[H_{\text{after}}\big]
$$

and each steerable sensor takes the facing maximising `gain`. Sensors with no
`for_width_deg` see all round and have nothing to choose.

**Why it is affordable.** The expensive part of a detection rate is the line-of-sight
walk, and **LOS does not depend on facing** — only the field-of-regard gate does. So the
per-cell rate is computed once per sensor with the arc removed, cached against the pose it
was built for, and each of the twelve candidate facings is then a cheap arc mask over that
raster. Without this, one epoch would cost a viewshed per facing per sensor.

**Carried sensors, and the cache key that pays for them.** A drone-mounted sensor moves
every tick, so an exact pose key would rebuild its raster every epoch and never hit — which
is why carried sensors were originally excluded from this layer altogether. That was wrong
in a specific way: *not finding anything is evidence*, negative information is the whole
point of the POMDP layer (§8.2), and the most mobile observer on the field was the one
asset excluded from it. A recce drone could fly the length of the map and leave its side's
belief unchanged.

They are now included, keyed on a pose **quantised to the coarse belief grid** (and to a
25 m altitude band). This is not a fudge: the raster *is* a coarse-grid object — every
entry is a rate at a coarse cell centre — so keying it on the coarse cell the sensor stands
in is consistent with the resolution the whole layer runs at. The cost becomes proportional
to how far the drone has flown rather than to how long it has been airborne. Quantisation
is integer arithmetic, so the rebuild schedule is identical on every run.

Emplaced sensors keep their **exact** pose as the key. They do not move, so the cache hits
every epoch after the first and there is nothing to buy by approximating — and V57 stays
pinned to the real geometry.

A carried sensor still has nothing to *steer*: it faces where its airframe points, and
`sync_carried_sensors` would overwrite any choice made here on the next tick. So it
contributes coverage without participating in the facing decision.

**Off by default (`[sim] sensor_tasking`).** A `facing_deg` written in a scenario is a
statement of intent, and silently overriding it would change what every existing scenario
means. It would also dissolve the §6.3 interdiction game, whose Blue strategies *are*
committed postures — a sensor that re-points itself is no longer playing a strategy. V39
caught exactly this when the default was briefly `true`.

*Measured*, on `scenarios/sensor_search.toml` (three 70°-arc observers, five Red units,
none of them in the sectors the observers start on): a fixed stare finds **2 of 5**; the
belief-driven sweep finds **5 of 5**. Nothing about the sweep is scripted — each sensor
drains its own belief out of ground it has cleared, so the best-information facing moves
on by itself.

### 10.4 Validation gates (V54–V58, V61)

| # | Property | Reference |
|---|----------|-----------|
| V54 | removal preserves history | removal tombstones rather than shifting: every index already in an event log still resolves to the same asset |
| V55 | track lifecycle & EW | a track lapses `track_hold_s` after its last observation and is cleared; continuous observation refreshes it indefinitely; jamming drives `λ_eff` below the maintenance threshold and so *breaks* a track, which permanent detection made impossible |
| V56 | allocation optimality | Hungarian matches an exhaustive brute-force optimum for n ≤ 7; its total payoff is never below greedy's; no target draws more shooters than it has slots; an ineligible pairing is never chosen |
| V57 | tasking beats staring | against an enemy hidden outside its initial arc, a belief-tasked sensor detects where a fixed stare never does, with a shorter mean time-to-detect; belief stays a normalised non-negative distribution with finite entropy across many updates (extends V42); tasking draws no randomness |
| V58 | decision-layer identity | with one shooter and one reachable target, every allocation rule and both tasking settings produce the identical detection and fire logs — the decision phases draw zero randomness, so the stream cannot shift |
| V61 | carried sensors inform belief | a recce drone that overflies ground and finds nothing drains its side's belief out of that ground, against a control with no drone; belief stays normalised; an emplaced-only scenario is unchanged (only carried poses are quantised); the cleared ground moves with the drone, so the raster is genuinely refreshed |

**Regression risk, as predicted and as found.** Allocation changes what units shoot at, so
V24, V30 (Lanchester), V31 and V39 were flagged as able to move. **V24, V30 and V31 did
not**, exactly as reasoned: they are single-shooter or homogeneous-line scenarios where
allocation degenerates to the old choice.

**V39 did move**, and for an instructive reason — not allocation, but *tasking*. Its Blue
strategies are committed sensor postures, so a sensor that re-points itself is not playing
a strategy at all, and the "unwatched lane" stopped being unwatched. That is what settled
`sensor_tasking` defaulting to off rather than on. The gate was doing its job: it caught a
model change that would otherwise have quietly invalidated the Phase 6 game.

### 10.5 Deferred

**Movement decisions in-loop** — re-pathing with `least_risk_path` against a live risk
raster. Out of scope for this phase: it needs a per-epoch risk raster, and the coarse-grid
machinery built for §10.3 is what makes that affordable later.
