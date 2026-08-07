[Index](README.md) · [← §6 Game-theoretic layer](06-game-theory.md) · [§8 Electronic warfare & partial observability →](08-ew-and-partial-observability.md)

---

## 7. The simulation loop

Written last, after four phases had each added to it. Every other section specifies a
*model*; this one specifies the **order those models run in and what that order
guarantees** — which is the thing most easily broken by accident and hardest to notice.

### 7.1 Two clocks, and why

The loop is hybrid continuous/discrete:

- **The tick** (`dt_s`, default 1 s) integrates what changes continuously — movement, and
  the moment-to-moment hazard of being seen.
- **The decision epoch** (`epoch_s`, default 10 s) is where *choices* are made: what is
  still being tracked, where to look, what to shoot.

The split is the optimal-control-plus-DP structure made concrete ([`MATHS.md`](../MATHS.md)
strands 1–2), and
it is load-bearing in both directions. Physically, fire missions are not re-planned sixty
times a minute. Computationally, it is what keeps the expensive decision layer — an
assignment solve and an information-gain search — off the hot path: §10.1 measured track
maintenance at ~2.3 ms per tick against ~0.23 ms at epoch cadence.

A tick may straddle an epoch boundary, or several. `step_one` advances the clock and then
resolves *every* boundary the new time has crossed, so `epoch_s` need not be a multiple of
`dt_s` and a coarse `dt_s` cannot silently skip a decision.

### 7.2 Phase order

The authoritative list. Phases 1–7 run every tick; phase 8 only on an epoch boundary.

| # | Phase | Draws RNG? | Spec |
|---|---|---|---|
| 1 | Ground movement along routes | no | §6.1 |
| 2 | Air movement, then carried-sensor sync | no | §9.2, §9.6 |
| 3 | Sensing vs enemy ground units | **yes** — one draw per eligible pair | §3.2 |
| 4 | Sensing vs enemy air | **yes** — one draw per eligible pair | §9.1 |
| 5 | Suppression recovery | **yes** — one draw per non-Free unit | §4.3 |
| 6 | Air-defence resolution | **yes** — per engagement due | §9.4 |
| 7 | Strike release | **yes** — burst point, damage rolls | §9.3 |
| 8a | Track maintenance | no | §10.1 |
| 8b | Sensor tasking | no | §10.3 |
| 8c | Fire allocation, then resolution | allocation no, rounds **yes** | §10.2, §2 |

Two orderings inside phase 8 are constraints, not preferences. **Tracks are maintained
before tasking**, because tasking reasons about what was *not* seen this epoch. **Tasking
precedes fires**, because indirect fire is gated on tracks — a sensor that loses contact
silences the guns behind it, and that must be visible in the same epoch.

Movement leads the tick so that sensing and fires act on current positions rather than
last tick's; a target that moved into cover this tick is in cover when it is looked at.

### 7.3 The determinism contract

**Same binary, same `(scenario, seed)` → bit-identical output.** Cross-platform
bit-equality is explicitly *not* promised (floats); float comparisons in gates use stated
tolerances.

Four structural rules make it hold, and each is enforced somewhere rather than trusted:

1. **One seeded stream.** All randomness comes from the `SimRng` the `Sim` owns
   (`ChaCha8Rng`, chosen because its stream is stable across `rand` versions — an archived
   seed must still reproduce after a routine dependency bump). No wall-clock, no thread
   RNG, no global state.
2. **Fixed iteration order.** Assets are visited by index, never by hash order; state
   carries no `HashMap` whose iteration reaches a result. Placement order is itself part
   of the contract (§10, `setup.rs`), so two runs agree on every index.
3. **Parallelism writes disjointly.** The rasters (viewshed, coverage, belief, risk) are
   computed with `rayon` via `ndarray::Zip`, where each cell writes its own slot and the
   LOS scratch is thread-local. No result depends on scheduling.
4. **New phases are appended and draw nothing when idle.** This is the rule that has let
   the loop grow from three phases to ten without invalidating older results.

### 7.4 The identity discipline

Rule 4 deserves stating as a design *method*, because it is how this project has added
every subsystem since Phase 3 without re-baselining what came before.

> A new subsystem must reduce to an **exact identity** when it has nothing to do — not an
> approximation, not "close enough". Switched off, the event log is bit-identical to the
> build before it existed.

Each such claim gets its own gate rather than being asserted:

| Subsystem | Identity when… | Gate |
|---|---|---|
| Electronic warfare | no jammers ⇒ every factor is exactly 1 | V40 |
| Air & counter-air | no air assets ⇒ phases 2, 4, 6, 7 draw zero RNG | V52 |
| Decision layer | one shooter, one reachable target ⇒ all rules agree | V58 |
| LOS memoisation | a cache hit is the value a miss would have computed | unit tests |

The payoff is concrete: adding drones did not move a single ground-scenario result, and
`sensor_tasking` could be added without touching the Phase 6 game — once it was defaulted
off, which V39 is what forced (§10.4).

The discipline also constrains *optimisation*, not just modelling. Both Phase-10 speed-ups
were verified by hashing a 4-scenario × 12-seed batch before and after and requiring the
digest to match. That is why the indirect damage factors are deliberately not
pre-multiplied into one term: float multiplication is not associative, and folding them
would shift a result by an ulp — enough to flip a knife-edge kill roll and silently
re-baseline V22 and V24.

### 7.5 What the loop does *not* do

Stated so the boundaries are visible rather than assumed:

- **No re-planning of movement.** Routes are fixed once given; a unit does not re-path
  around a threat that appears mid-run. Deferred as §10.5.
- **No intra-epoch reaction.** Both sides allocate against the board as it stood at the
  start of the epoch, so neither reacts to casualties the other has not taken yet. This is
  a deliberate simultaneity choice, not an oversight.
- **No variable time step.** A finer `dt_s` costs proportionally more; because detection
  is modelled as a rate rather than a per-tick probability (§3.2), it buys no accuracy in
  the detection statistics, which is exactly what V17 checks.

### 7.6 The input contract

The schema's `deny_unknown_fields` refuses a **key** the model does not know, on the
grounds that a misspelt dial takes its default and produces a study of a different
question — a failure that is invisible because the run succeeds. A **value** outside its
domain fails in exactly the same way, and until V67 nothing checked one.

`Scenario::validate` and `Libraries::validate` now refuse both, naming the offending dial.
Two of the refusals are of a different order from the rest, and are the reason the section
exists:

```
dt_s    = 0   ⇒  the clock never advances, so `run_until` never returns
epoch_s = 0   ⇒  time_s / epoch_s is +∞, and `as u64` **saturates** rather than wrapping,
                 so the epoch loop is handed u64::MAX boundaries to resolve
```

Neither is exotic. `experiments/sweep` exists precisely to set any dotted path in a file
from the command line, so `--param sim.epoch_s --from 0 --to 30` is an ordinary-looking
sweep whose first arm hangs with no diagnostic.

The rest are ordinary domain checks — probabilities in `[0, 1]`, durations and radii
non-negative, `belief_cells ≥ 1` — plus the small set of stat-block dials that reach a
**divisor**: a sensor's `range_half_m` in the §3.2 falloff, an indirect weapon's
`lethal_radius_m` in the §2.3 Carleton kernel. Those two are singled out because a zero
there does not give a small answer, it gives `NaN`, and `NaN` loses every comparison it
appears in — so the subsystem goes silently *inert* rather than visibly wrong, which is
the hardest kind of failure to notice.

**The list is deliberately short.** Most dials being zero is a legitimate statement, and
refusing them would break real fixtures: a drone with `cruise_speed_m_s = 0` is
stationary, which V59 and V62 depend on to keep their geometry from drifting; a battery
with `max_range_m = 0` engages nothing; a direct weapon never touches the Carleton kernel,
so its unused `lethal_radius_m` of zero means nothing at all. A validator that refused
those would be enforcing taste rather than tractability.

`Libraries::validate` runs both at load and again inside `Sim::new`, so a library patched
in memory — which is what `sweep` does — is held to the same contract as one read from
disk.
