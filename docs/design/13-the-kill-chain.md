[Index](README.md) · [← §12 SEAD: air defence as a target](12-sead.md)

---

## 13. The kill chain: directed targeting

§10.2 allocates fire by maximising `P(kill) × value`. That is what an **omniscient
optimiser** would do, and as a bound on how well a side could possibly shoot it is exactly
the right thing to compute.

It is not how a force fights. A gun crew does not hold a kill-probability table. It holds
**orders** - engage air defence before manoeuvre, shoot the command post first, counter-
battery takes precedence - and it follows them whether or not the shot in front of it is a
good one. So a declared priority here is **strict by default**: a shooter that can reach
anything in a higher tier takes it, even at a worse kill probability than a lower tier
offers.

That is not a crude approximation of the optimiser. It is a different decision rule, and
for a directed force a more faithful one. Which turns the mode switch into a measurement
rather than a preference: running the same scenario under strict doctrine and under the
payoff-optimal allocation puts a number on **what directive control costs against optimal
control**. Compare with §10.2, which measured what *no* control costs against optimal - the
three together bracket the question.

```
sweep <scn> --param blue.doctrine.mode --values strict,weighted --seeds 1000 \
      --metric red_cleared_s
```

### 13.1 What a priority entry may name

Four things, checked in this order, all equally valid:

| Entry | Matches |
|---|---|
| an asset **id** | that one asset - how a gate pins an exact target |
| a **role** | every asset whose stat block declares it (`role = "artillery"`) |
| a **class** | `unit`, `air_defence`, `c2`, `air` - always available, never declared |
| `"all"` | anything - the tier that says "and then everyone else, equally" |

**There is no "no doctrine".** A side always has one; omitting the block gives
`priority = ["all"]`, a single tier holding every target and ranked among itself by the
ordinary payoff - which *is* the undirected behaviour. So the engine has **one** code path
rather than two, and the identity with the pre-doctrine model holds by construction (one
tier means one solve over every target, exactly what the old code did) rather than by a
separate branch that has to be kept honest. `"all"` is usable mid-list too, which makes the
bottom tier explicit: `["c2", "air_defence", "all"]` reads as the fire plan it is.

Roles are free-form strings on the stat block, so a scenario can invent whatever
categories it needs; adding "engineer" takes no code change. A role never *masks* its
class - a battery with `role = "sam"` matches both `"sam"` and `"air_defence"` - so a
coarse doctrine keeps working when a stat block later becomes more specific.

The **first** matching entry decides the tier, which is what lets a list single one asset
out and then name the class beneath it: `["sam-1", "air_defence"]` puts that launcher a
tier above its own siblings. Anything unnamed falls into an implicit bottom tier, still
ranked among itself by the ordinary payoff.

**Every name is checked when the sim is built.** A priority entry matching nothing is a
load error listing what would have worked, not an empty tier. Same reasoning as the
schema's `deny_unknown_fields`: a tier that silently matches nothing fails invisibly - the
run succeeds and simply answers a different question from the one asked.

### 13.2 How a priority is applied

| Mode | Rule |
|---|---|
| `strict` (default) | tier decides; the payoff only breaks ties *within* a tier |
| `weighted` | tier scales the target's value by `falloff^-k`; the payoff still decides |

**Strict is implemented by solving the assignment one tier at a time**, highest first. Any
shooter that can reach a tier takes something in it; only those left unassigned fall
through to the next. That makes the ordering *exact*.

The obvious alternative - add a large bonus to a higher tier's payoff - is a trap this
codebase has already fallen into once. `allocation::INELIGIBLE` was originally `-1e18`, and
at that magnitude `1e18 + 10.0 == 1e18` in `f64`, so every matching with the same forbidden-
cell count scored identically. A tier bonus large enough to dominate would swallow the
payoff differences *inside* a tier the same way. A sequence of small exact problems has no
such failure mode.

Doctrine applies to **ground fires and air defence** alike, and on both the coordinated and
the uncoordinated paths. Being outside the C2 net (§11.3) costs a battery its coordination,
not its orders: a lone gun still shoots what it was told to shoot first, it simply does not
know what anyone else is shooting.

### 13.3 Ordered engagements

```toml
[[blue.orders]]
shooter = "gun-a"
target  = "sam-1"
```

The bluntest instrument here, and the one a gate usually wants. An ordered shooter is
removed from the assignment problem entirely, so "gun-a engages sam-1" is a *fact about the
run* rather than a likely outcome of it. Everything not under orders is allocated normally,
so a scenario can pin one pairing and let the rest be solved.

Both ends must still be alive; an order against a destroyed target lapses and the shooter
rejoins the problem, because a standing order does not make a crew fire at a wreck. Range
and line of sight are **not** checked - an order is an order, and a shooter that cannot
reach its target wastes the epoch. That is the cost of giving a bad one, and it should be
visible rather than silently corrected.

### 13.4 Eligibility blocks, and a shooter holds what it takes

Two rules keep a fire plan from becoming a way to waste ammunition.

**Line of sight and range block a pairing; they do not merely lower its score.** A target a
shooter cannot engage is `INELIGIBLE` in the payoff and is never returned by any solver, so
a shooter whose whole top tier is masked by a ridge finds nothing there and **falls through
to the next tier**. It is never left idle facing a hill while something it could engage goes
unengaged. `Sim::can_engage` is the one test - alive, in range, in line of sight for direct
fire, holding a live track for indirect - and doctrine, target locks and ordered engagements
all ask it, so all three agree about what "reachable" means.

That applies to `[[orders]]` too. An order stands while the pairing is reachable and lapses
while it is not, resuming the moment the target reappears. Making orders the exception -
"an order is an order, the crew tries anyway" - is defensible in isolation but contradicts
the rule everything else follows, and it would let one bad order silence a gun indefinitely.

**A shooter holds its target.** `UnitState.engaging` is a **lock**: once taken, it is kept
until the target is dead or can no longer be engaged. Air defence has always worked this way
(`AirDefenceState::engagements`, dropped by `drop_engagements` when the target leaves the
envelope); this is the ground half of the same idea.

Without it a gun re-decides from scratch every epoch and flip-flops between two
near-identical targets as tiny payoff differences wobble - wasted fire for a reason no crew
would recognise. Switching targets is itself a decision with a cost, so it takes something
changing on the ground, not a rounding difference. The one thing that *does* break a lock is
a new order, which is the point of an order.

A held lock still counts against its target's **discount**. Otherwise the $(1-\bar q)^k$
sequence would restart for every shooter that happened to be re-deciding, and a target
already covered by three locked guns would look as attractive to a fourth as an untouched
one.

### 13.5 Deliberate limitations (v1)

- **Doctrine is static.** A side's priority does not change with the situation; there is no
  "switch to counter-battery once the air threat is gone". A conditional kill chain is the
  natural next step and would sit on the same tier machinery.
- **Priority is not per shooter.** The whole side shares one list. Giving a specific
  battery its own priority would need doctrine on the instance rather than the force.
- **Strike drones are unaffected.** They still attack the asset a scenario named (§12.5);
  doctrine drives *allocation*, and a strike drone does not allocate.

### 13.6 Validation gates (V66)

| # | Property | Reference |
|---|----------|-----------|
| V66 | directed targeting | line of sight and range **block** a pairing, so a masked priority target does not hold a shooter hostage - doctrine and orders alike fall through to what can be engaged; a shooter **holds** its target until it is dead or unengageable, and a held lock consumes a slot so the overkill cap cannot be bypassed; a gun with a 46% shot at a near high-value tank and a 3% shot at a far SAM takes the tank unprompted and the **SAM** when told `priority = ["air_defence"]` - strict doctrine is followed, not weighed; the same priority in `weighted` mode does not overturn the fifteen-fold better shot; an ordered engagement bypasses the assignment and lapses if its target dies; no doctrine is an exact identity (§7.4); a priority naming nothing is a load error listing what would have worked, and ids, roles and classes are all accepted; air defence follows the same doctrine, taking a further strike drone over a nearer recce one |
