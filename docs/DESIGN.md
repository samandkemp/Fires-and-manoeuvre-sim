# Build log

How this model was built, in the order it happened, and why each decision went the way it
did. The other documents describe what the simulation *is*; this one records how it came to
be that way - the choices taken, the things that turned out wrong, and what was learned from
correcting them.

It is kept because a model's history is not recoverable from its current state. A page of
equations cannot say which of them replaced an earlier attempt, which limitation was
accepted deliberately, or which finding was measured twice before it was believed.

**The specification lives elsewhere.** The `docs/DESIGN.md §N.M` references throughout the
source resolve to [`docs/design/`](design/), one page per section, indexed in
[`docs/design/README.md`](design/README.md).

| § | Page |
|---|---|
| 0 | [The strand index](design/00-strand-index.md) |
| 1 | [Terrain & LOS](design/01-terrain-and-los.md) |
| 2 | [Fires](design/02-fires.md) |
| 3 | [Sensing & detection](design/03-sensing.md) |
| 4 | [Suppression & attrition](design/04-suppression-and-attrition.md) |
| 5 | [Movement as DP](design/05-movement-as-dp.md) |
| 6 | [Game-theoretic layer](design/06-game-theory.md) |
| 7 | [The simulation loop](design/07-the-simulation-loop.md) |
| 8 | [Electronic warfare & partial observability](design/08-ew-and-partial-observability.md) |
| 9 | [Air: drones & counter-air](design/09-air-and-counter-air.md) |
| 10 | [The decision layer](design/10-the-decision-layer.md) |
| 11 | [Command and control](design/11-command-and-control.md) |
| 12 | [SEAD: air defence as a target](design/12-sead.md) |
| 13 | [The kill chain: directed targeting](design/13-the-kill-chain.md) |

---

## The order it was built

Phases are not sections. A later phase usually amends earlier ones rather than adding a
section of its own, so the two numbering schemes agree up to §12 and part company after it.

| Phase | What it added | Where it landed |
|---|---|---|
| 1 | Terrain, derived layers, LOS, viewshed | §1 |
| 2 | Sensing and detection, deliberately ordered *before* fires | §3 |
| 3 | Direct and indirect fires | §2 |
| 4 | Units as elements, suppression, attrition | §4 |
| 5 | Movement as dynamic programming | §5 |
| 6 | The game-theoretic layer | §6 |
| 7 | Visualisation | app only |
| 8 | Electronic warfare and the belief filter | §8 |
| 9 | Air, counter-air, and the slant-range convention | §9, and §1.2 for height |
| 10 | The decision layer: tracks, allocation, tasking | §10 |
| 11 | Command and control | §11 |
| 12 | SEAD, making air defence attritable | §12 |
| 13 | Closing the §11/§12 gaps | amended §10.4, §11.2-§11.6, §12.3-§12.6 |
| 14 | The kill chain: directed targeting | §13 |
| 15 | Closing the open model questions | amended §2, §11.4, §12.5 |
| 16 | Measurement machinery: factorial designs, global sensitivity | tooling |
| 17 | Movement decisions in the loop | §10.5 |

**§13 is the work of Phase 14**, not Phase 13. Phase 13 lifted five limitations across three
existing sections and so has no section of its own.

---

## Decisions worth recording

### Sensing before fires

The original order put fires third and sensing second. It was swapped deliberately: sensing
is the centrepiece of the tool and the visual heart of it, so landing it earlier meant the
interactive loop existed sooner, and it hardened the LOS interface while changing that
interface was still cheap.

### Slant range as the project-wide convention

Adopted when air arrived, because a drone at altitude makes horizontal range meaningless.
The reason it needed no re-baselining of the existing gates is that on flat ground with
equal actor heights it reduces to the old distance *exactly*, so every previously measured
result was already a slant-range result.

### Every subsystem must reduce to an exact identity when switched off

The single discipline that made eight further phases safe to add. A scenario with no
aircraft produces the event log it produced before the air model existed, byte for byte.
This is stronger than "approximately unchanged" and deliberately so: an approximate identity
hides a real change inside sampling noise, and there is no threshold at which it can be
distinguished from a bug.

Where possible the identity is **structural** rather than dial-gated, meaning the new code
does not run at all rather than running with its effect set to zero. Movement decisions
(§10.5) build no planner when no unit declares an objective; doctrine defaults to a single
tier over everything, which *is* the undirected behaviour.

### Stats are data, and a dial the model cannot run on is refused at load

The same argument as rejecting an unknown field: a bad value does not crash, it silently
answers a different question. The list of validated dials is deliberately short - only where
a value reaches a divisor or unbounds a loop - because a zero is usually a legitimate
statement, such as a stationary drone or a battery that engages nothing.

### Coordination is an asset, not a dial

Command and control could have been a boolean on a side. Making it a placed, killable,
jammable post is what allows "suppress the enemy's headquarters" to be compared against
"kill one more launcher", and it is the seam SEAD later hung off.

### Doctrine is strict by default

A crew follows orders, not a kill-probability table. Making strict ordering the default
means the payoff-optimal allocation becomes the *idealised bound* rather than the assumed
behaviour, so the mode switch measures what directive control costs against optimal control.
Implemented by solving one tier at a time rather than by a large payoff bonus, which would
have reintroduced the floating-point precision trap that a sentinel `-1e18` ineligibility
value had already caused once.

### Planning happens on a coarse grid

Forced by measurement, not preference. A risk raster at full terrain resolution costs about
4 seconds for a 1000x1000 map with two sensors, which at one decision epoch per 10 seconds
is roughly a hundred times the cost of the rest of the simulation. The same coarse grid the
belief layer uses was reused, on the reasoning that a commander choosing an approach every
ten seconds is not choosing between adjacent 10 metre cells.

---

## Things that turned out wrong

### The allocation finding, wrong twice in opposite directions

The longest-running error in the project, and the most instructive.

It first claimed the greedy allocator *beat* the optimal solver "consistently and outside
the noise". It did not. That came from comparing two unpaired means, at 30 and 200 seeds,
whose difference sat inside the sampling error, with no standard error reported to make that
visible.

Corrected, it then claimed no effect: a paired difference of -0.12 seconds with a standard
error of 0.22. That was correct for the model **as it stood**.

Phase 15 then removed the hard ground overkill cap, changing how many shooters may pile onto
one target - precisely the freedom the assignment objective ranges over. The difference
resolved into a significant one: optimal is **0.405 +- 0.051 seconds worse** than greedy
(t = 8.0 at 2,000 seeds, the two agreeing on 96% of seeds). The old figure was not
mismeasured; it was invalidated by a model change, and nothing re-ran it for two phases.

The solver is not at fault and its gate still holds: the Hungarian result *is* the optimum
of the objective it is given. The objective is the problem, because it scores a single
epoch, so solving it exactly is myopically right and can cost over a whole engagement
against a greedy rule that happens to spread fire. Optimising a surrogate harder does not
improve what the surrogate stands for.

Two habits came out of it. Report a standard error on every figure and pair every
comparison. And compare two arms **against each other**, never by eye across a shared
baseline: measured directly the gap is 0.405 +- 0.051, but read off two separate baselines
it looks like 0.4 against standard errors of 0.23, five times noisier, which is how a
significant effect stayed hidden.

### A side could inherit the enemy's fire plan

Coordinated air defence pooled *both* sides' batteries into one assignment and took the
doctrine from whichever battery came first in the list. A side that fielded a command post
could therefore inherit the enemy's fire plan, and which side that was depended on nothing
but the order units were written in the scenario file. No shipped scenario had coordinated
air defence on both sides, which is why nothing caught it. The gate now includes a mirrored
two-sided case that fails without the fix.

### Two clock dials could hang the simulation

A zero decision-epoch length makes the epoch count infinite, and the conversion to an
integer *saturates* rather than overflowing, so the epoch loop was handed the maximum
possible boundary and did not terminate. Both were reachable from an ordinary parameter
sweep starting at zero. This produced the input contract.

### The overkill cap was enforced in the wrong scope

It applied per fire-control problem rather than per side, so splitting a side into netted
and loose shooters applied the cap twice and a *divided* side fought measurably better. The
cap was also hard, idling shooters rather than discouraging them. It was replaced by a soft
discount, since the existing slot discount already prices piling on.

### One flag carried two meanings

A single `self_cue` flag meant both "this battery cues itself" and "this battery is
emitting". A battery set not to self-cue therefore counted as silent to an anti-radiation
missile while still detecting normally - survivability without the corresponding blindness.
Splitting the flag in two fixed it.

### A demonstration scenario that measured nothing

Written to show movement decisions and then withdrawn. A diagnostic confirmed the planner
working, but the scenario's geometry never let the choice matter: the observer either saw
the whole map, saw nothing, or the crossing did not finish inside the run. A scenario that
measures nothing is worse than no scenario, because it looks like evidence.

### An anti-radiation seeker that was never tested

Its carrier first released at 1500 metres, inside a defending gun's 2000 metre envelope, and
was shot down before releasing on 99.6% of seeds. Both arms of the comparison read zero and
the seeker was never exercised. Release range and engagement envelope are a matched pair.

---

## Limitations that were later lifted

| Limitation | Lifted by |
|---|---|
| Air-defence sites are not attritable | §12: batteries and posts take area damage like units |
| Detection of air is permanent | §10.1: air tracks decay like ground tracks |
| The command post is not attritable | §12: posts carry elements and can be destroyed |
| The overkill cap is scoped to a fire-control problem, not a side | §11.4 |
| `self_cue` carries two meanings | §12.5, by splitting the flag |
| Carried sensors build no coverage, so a recce drone's negative information never reaches belief | §10.4 |
| Movement decisions are not in the loop | §10.5 |

Carried sensors were originally excluded from the belief layer because a moving raster would
never hit its cache. That reasoning was wrong: keying on a pose quantised to the coarse
belief grid makes the cache work, and the raster was already a coarse-grid object, so this
is consistent rather than a workaround.

---

## Performance work, and what it cost

Optimisation was deferred until each subsystem was validated, and every change below was
verified bit-identical against a multi-scenario, multi-seed hash rather than trusted.

- Terrain hill-sum fields evaluate in parallel, and the woodland hill count is capped,
  because it scaled with map area and one preset wanted 1.1 billion evaluations. Build time
  for the largest map fell from 13.8 to 3.2 seconds.
- Line-of-sight became allocation-free, and its breakpoint sort was replaced by a linear
  merge of two monotone crossing streams.
- Line of sight is memoised per sensor-target pair, reused only on *exact* endpoint
  equality, which took the simulation tick from roughly 105 to 9-14 microseconds.
- Batch runs reuse terrain between trials, which also stopped map variance and dice variance
  being averaged together - a correctness gain that arrived disguised as a speed-up.

The simulation tick is now sub-millisecond and too noisy to optimise against. Terrain
*build* time is the figure that matters.

---

## Housekeeping decisions

**Documentation was split one job per document** because the introduction had carried three
hundred lines of mathematics before saying what the project was, and the specification was a
single page of more than sixteen hundred lines.

**Section numbers are never renumbered.** They are referenced from roughly three hundred
places in the source, so the numbering is load-bearing.

**Three experiment binaries were deleted** once the shared study harness subsumed them. None
of the bespoke binaries had ever used that harness, all predating it, which is how they went
stale without anyone noticing.

**Validation suites are grouped into fewer test binaries.** Each `tests/*.rs` file is its own
link unit, so twenty-nine of them made every engine edit expensive. The saving was smaller
than predicted, 13% rather than 45%, because most of the time is compiling the engine once
regardless; the benefit that did land was navigability.
