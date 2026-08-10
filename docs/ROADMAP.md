# Roadmap

Phases 1–14 built the model; the review pass hardened it. This is what comes next, in the
order agreed, with the decisions already taken recorded against each.

Gates continue from **V67**. Every phase keeps the two standing rules: a new subsystem is
*appended* to the loop and draws zero randomness when its inputs are empty, and it ships
with a gate against a closed form or a stated invariant.

| Phase | What | Why it is in this position |
|---|---|---|
| ~~15 (A)~~ | ~~Close the open model questions~~ | **Done.** Gates V68–V70; the two findings it corrected are re-measured in place |
| ~~16 (D)~~ | ~~Measurement machinery~~ | **Done.** `factorial`, `sensitivity` (gate V71) and quantile intervals |
| ~~17 (B)~~ | ~~Movement decisions in-loop~~ | **Done.** Gates V72–V74; a demonstration scenario is still outstanding (§10.6) |
| **18 (C)** | The dynamic game | The capstone. Needs 17 first: value functions must exist in-loop before they can be payoffs |
| — | Fidelity breadth | **Paused.** See the note at the end |

---

## Phase 15 (A) — Close the open questions *(complete)*

### A1. The overkill cap becomes soft *(decided: soft cap)*

`max_shooters_per_target` is a **hard** cap that *idles* shooters, and it is enforced per
fire-control problem rather than per side. Measured on `fires_c2.toml`: requiring a C2 net
makes Blue clear Red **24.5 s faster** (t = −10.0), because splitting a side applies the cap
twice and the coordinated side leaves guns doing nothing while the split side puts them to
work. Sweeping the net radius is monotone — 0/2/3 guns netted gives 68.98 / 78.86 / 103.40 s,
the last being exactly the no-net baseline.

§10.2 already discounts the *k*-th shooter on a target by $(1-\bar q)^k$, so piling on is
priced. A hard cap on top truncates that rather than discouraging it, and a shooter with
nothing else to engage does nothing instead of contributing at a discount.

**Steps** — drop the hard cap from the slot construction in `sim/engagement.rs`; let the
geometric discount do the work; re-measure everything the old cap touched.

**Gate V68** — no shooter idles while it has an engageable target. That is the property the
current rule violates.

**Re-measure:** `fires_c2`'s 2×2 and net-radius sweep, and §11.2's finding that *"cap 2 buys
nothing and cap 3 is actively worse"* — which was measured under the hard cap and may not
survive it. If the result changes, the new one is the true one.

### A2. Split `self_cue` in two

§9.5 introduced `self_cue` as the **cueing timeline** ("this battery is cued from elsewhere
and pays `cue_latency_s`"). §12.3 reused the same flag as the **emission** test for
anti-radiation homing. So a `self_cue = false` battery counts as silent to a missile while
its radar keeps detecting perfectly well — measured on `sead_arm.toml` at 1.000 detections,
first contact 9.3 s, indistinguishable from the emitting arm. It gets the survivability of
EMCON without the blindness that should pay for it.

**Steps** — add `emitting` (default `true`) to a placed battery; `target_is_emitting` and
`sensor_active` both key on it, so a silent battery stops detecting; `self_cue` retains only
its §9.5 meaning.

**Gate V69** — the two flags are independent. `emitting = false` is invisible to an ARM
**and** contributes no detections; `emitting = true, self_cue = false` detects normally but
pays the cue latency. Both defaults are exact identities.

### A3. Gate the indirect half of eligibility

V66 established that line of sight and range **block** a pairing, so a masked priority target
falls through rather than holding a shooter hostage. Its fixture only exercises **direct**
fire. Indirect fire needs no sightline — it needs a live track — so an indirect shooter's
tier falls through, and its lock breaks, on `track_hold_s` rather than on terrain.

**Gate V70** — asserts that asymmetry in both directions. No code change is expected; if it
fails, that is a find worth having.

### A4. Crest clearance

Indirect fire has no minimum-ordinate check, so a gun immediately behind a high mask still
lands rounds. A deliberate simplification, but it belongs in §2's limitations rather than
being discovered.

---

## Phase 16 (D) — Measurement machinery *(complete)*

### D1. Factorial designs

`sweep` varies one dial. The `fires_c2` investigation needed a 2×2 and it was hand-stitched
from four sweeps — and the *interaction* turned out to be the dominant effect. That is an
argument that interactions are where the findings are.

**Steps** — build the Cartesian product of dial levels; run every cell over the shared seed
set, paired as always; report main effects and two-way interactions with standard errors.

**Harness tests** — a one-factor factorial reproduces `sweep` bit-identically; a factor with
no effect returns an interaction within noise.

### D2. Global sensitivity analysis *(decided: its own tool)*

Every number in this repository is a placeholder, so the loudest unanswered question is
which conclusions survive the dials being wrong.

A Sobol study is a different object from a paired comparison — different sampling (Saltelli,
not a shared seed set), different output (variance decomposition, not a difference with an
error bar), different question. So it gets **its own spec file, binary and report** rather
than being folded into `sweep`, where one report format would have to serve two incompatible
purposes.

**Steps** — a `studies/*.toml` naming dial paths and ranges; Morris elementary effects for
cheap screening; Sobol first-order and total-effect indices via Saltelli sampling.

**Gate V71** — the estimator recovers the **analytic** Sobol indices of the Ishigami
function. A genuine closed form, so this earns a V-number rather than a harness test.

### D3. Confidence intervals on quantiles

Means carry error bars; tail metrics — leakers, worst-case clear time — do not. Bootstrap
CIs, tested against a distribution with known quantiles.

---

## Phase 17 (B) — Movement decisions in-loop *(complete)*

`least_risk_path` is called only from `experiments/` and `validation/`. The dynamic
programming strand sits *beside* the model rather than inside it, and movement is the one
decision still scripted: fires are allocated and sensors are tasked, but routes are drawn by
hand.

### The shape *(decided: per-unit objective, not a global dial)*

A unit either has a `route` — scripted, exactly as today — or an `objective`, which it
optimises toward against the live risk raster. Declaring both is a load error.

This is deliberately **not** a `[sim]` switch. With a per-unit objective the identity holds
**by construction**: a scenario with no `objective` does no new work at all, rather than
having a branch turned off. Same argument that made doctrine's `priority = ["all"]` default
better than an `Option`.

It also buys what a global dial could not: a scripted unit and an optimising unit **on the
same map, on the same seed** — control and treatment in one trial.

### Steps

1. **Per-side risk raster**, on the coarse grid `sim/tasking.rs` already uses for belief.
   `risk(cell)` is §5.2's definition — the detection rate a reference mover would suffer from
   the best-placed enemy sensor. Cached with exact-key invalidation, as `sim/los_cache.rs`
   does.
2. **The re-path decision** at each epoch for units with an objective.
3. **Per-unit `risk_weight`**, defaulting from `[sim]`. This is §5.1's exchange rate $w$, and
   sweeping it traces the Pareto frontier between arriving quickly and arriving alive — the
   experiment worth running, now a one-line sweep.

**The trap, named in advance:** a unit re-deciding every epoch will dither between two
near-equal paths. That is the movement analogue of the target-lock problem, and it gets the
same answer — adopt a new path only if it beats the held one by `repath_margin`.

**Gates**

- **V72** — structural identity: a scenario with no `objective` is bit-identical, and the
  phase draws zero randomness.
- **V73** — a unit re-paths around a newly placed sensor's coverage; with `risk_weight = 0`
  it takes the shortest path regardless, recovering V25.
- **V74** — no oscillation under a static risk field.
- Declaring both `route` and `objective` is a load error (V67's family).

**Risk:** V37, V38 and V39 all involve moving units, and the interdiction game's premise is a
committed route. The per-unit design is the mitigation; V72 is what proves it.

---

## Phase 18 (C) — The dynamic game

The least specified of the phases, and the steps below are as much open questions as tasks.
Needs Phase 17 first.

1. **Value extraction** — per side, per epoch, from the DP sweep.
2. **State abstraction** — the hard part, and the real modelling work. Raw positions are
   intractable; something like (belief summary, force ratio, posture) is needed, and
   choosing it *is* the contribution.
3. **Payoffs from value functions** rather than Monte Carlo battles.
4. **Solve** — fictitious play over the abstracted state, or a stage game per epoch carrying
   continuation values.

**Gates** — **V75**, with a single abstract state the dynamic solver reduces *exactly* to
today's static `solve_zero_sum`; **V76**, a game with known value embedded in the state space
recovers that value.

Expect a design phase before any code. The risk here is specification, not implementation.

---

## Paused: fidelity breadth

Acoustic detection, air-to-air, and terrain-aware comms remain open but are not scheduled.
Each is self-contained and none blocks anything else, so they can be picked up on curiosity.

**Real-terrain (DEM) ingestion is dropped, not deferred.** The project's terrain decision is
synthetic-indefinitely, and the point of `TerrainSource::Layers` is that a map can be
*described* — a base surface plus ordered ridge, woodland and urban features. Plausible but
fictitious ground serves the operational-research purpose better than real ground, because
you can construct the ridge that makes a scenario bind rather than hunting for one.

**Terrain-aware comms is not really fidelity breadth** — it is the §11.1 limitation the C2
model already names, and belongs as a small follow-on to whichever phase next touches C2.
