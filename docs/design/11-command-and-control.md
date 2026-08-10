[Index](README.md) · [← §10 The decision layer](10-the-decision-layer.md) · [§12 SEAD: air defence as a target →](12-sead.md)

---

## 11. Command and control *(Phase 11)*

Ground fires coordinate side-wide for free (§10.2) — defensible for a battlegroup sharing
one fire-control net. Air defence should not, and did not: each battery independently
engaged whatever was nearest.

Decision (user, 2026-08-05): **coordination is an asset you field, not a switch you set.**
A dial would have made "the batteries cooperate" free and permanent. Making it a placed
**C2 post** makes it something that must be paid for, positioned, and can be taken away —
which is the behaviour worth modelling, and the seam SEAD hangs off.

### 11.1 The C2 post

A post is a placed asset with a `coordination_range_m`. Batteries within that radius of a
**live friendly** post allocate as one group; batteries outside act on their own. It has
no weapon, no sensor, and does not move. Its only effect is on who is in whose assignment
problem.

Range is **horizontal**, not slant (§9.1). A coordination link is a communications
relationship, not a sightline; using slant range would make a post on a hill mysteriously
worse at talking to the battery beneath it. A terrain-aware comms model is a clean later
refinement — the §9.5 cue-latency machinery is the natural seam.

Destroying a post costs no battery, no magazine and no envelope. What is lost is the
coordination: from the next tick the group **decoheres** and every battery reverts to
nearest-first. That is what makes "kill the command post" a better opening move than "kill
one more launcher", and the model produces it without being told to.

**The link is not all-or-nothing.** Two things bear on it besides the post being alive.

*Jamming pulls the radius in.* The effective coordination range is

$$
r_{\text{eff}} = r_{\text{coord}} \cdot g(\text{post})
$$

where `r_coord` is `coordination_range_m` and `g` is `ew::jamming_factor` evaluated at the
post. So an enemy jammer near the post does not flip the link off — it shrinks it, and the
batteries on the flanks fall out of the net while the one sitting on top of the post keeps
talking. That is the right shape: a link degrades with range against a noise floor, and
raising the floor is what a jammer does. It also gives the raid a **soft** counter beside
SEAD's hard one — same effect on the defence, no ordnance spent, and nothing on the map to
show it happened.

Note the sign. `Sim::jamming_at` folds a side's **own** jammers, because a jammer protecting
Red degrades *Blue's sensing of Red*. `Sim::link_quality_at` folds the **enemy's**, because
a jammer degrades *Blue's own communications*. Same asset, same dials, opposite side of the
argument — so one Red jammer both hides Red units and cuts the Blue net.

*Joining costs time.* `link_latency_s` (default **0**) is how long a battery must have been
inside the radius before it is in the net. Defaulting to zero recovers the pre-latency
behaviour exactly, and — the reason it matters for study design — lets a sweep turn either
effect on **alone**, without the other confounding it. The consequence worth noting is that
a battery not yet in the net falls back to nearest-first and commits its channel: a link
that arrives late cannot retrospectively undo the duplicated engagements already made.

### 11.2 The air-defence payoff

`max_batteries_per_air_target` (default 2) is the only overkill cap left in the model. The
ground side has none: §11.4 removed it because a hard cap *idles* shooters, and a
multi-element ground unit genuinely absorbs several of them, so the $(1-\bar q)^k$ discount
is the whole story there.

An airframe is different, and that is why the dial survives. It is a **single object**, so a
second battery is insurance against the first missing rather than extra damage delivered,
and a missile is a discrete round out of a finite magazine rather than a continuous rate.
There is a real quantity to be wasted, which is what a cap is for.

**Measured (1,000 paired trials on `ad_c2`).** That reasoning was still a claim, so it was
swept. Against a cap of 1:

> The same study is written up as a *worked example* in
> [EXPERIMENTS.md](../EXPERIMENTS.md#worked-example-is-the-overkill-cap-earning-its-keep),
> where the point is how to design and read it rather than what it says about the model.
> **Both carry the numbers, so re-measuring means updating both.**

| cap | drones downed | rounds left |
|---|---|---|
| 2 | +0.013 ± 0.011 (t = 1.2) — **not significant** | −0.233 ± 0.064 (t = −3.6) |
| 3 | −0.020 ± 0.012 (t = −1.6) — **not significant** | −0.584 ± 0.067 (t = −8.7) |

So on this scenario the second battery buys **nothing measurable** and costs about a
quarter of a round; the third buys nothing and costs more than half a round. Neither is a
disaster and neither is an argument for the default — it is defensible but unearned here.
Whether a cap above 1 earns its keep when batteries are scarce relative to the raid is the
open question, and it is a question this scenario cannot answer because it has three
batteries and ten drones.

Reproduce with:

```
sweep ad_c2 --param sim.max_batteries_per_air_target --values 1,2,3 --seeds 1000 \
      --until 240 --metric ad_rounds_left
```

Rows of the assignment are **free engagement channels**, not batteries — a two-channel
battery contributes two rows, so `channels` falls out of the structure rather than needing
a special case. Columns are slots on each engageable airframe, discounted geometrically as
in §10.2.

**One assignment per side.** A post coordinates its own batteries and nobody else's, so
when both sides are coordinated there are two problems, not one. This is not a refinement:
solving them together has no well-defined answer, because a pooled group has no single
doctrine to be scored under and no single `max_batteries_per_air_target` budget to spend.
The implementation originally pooled them and took the side from the first battery in the
list, which meant a side that fielded a post could inherit the *enemy's* fire plan — and,
because the list is built in placement order, which side that happened to be depended on
nothing more than who was written first in the scenario file. V59 now fields a mirrored
two-sided engagement and requires each side to make the same choice it would make with the
enemy's post removed.

$$
\text{payoff}\big[\text{channel}\big]\big[(\text{air}, k)\big] =
P(\text{kill before release}) \cdot \text{value}(\text{air}) \cdot (1 - p)^{k}
$$

**The deadline is the release point, not the envelope edge.** A drone that leaves the
envelope having already dropped its munition has won, so the window is the time to reach
`release_range_m` of its aim point, and the battery best placed to stop the airframe
*closest to doing damage* wins it — not the one that happens to be nearest. An airframe
with nothing left to drop has no such deadline and is scored over the time to cross the
envelope instead: still worth shooting, just not urgently.

The window is capped at a **planning horizon** of 60 s. Two reasons, the second concrete:
beyond about a minute, "how long this target will linger" stops discriminating usefully,
since the defence will have reconsidered many times; and an uncapped window runs to
hundreds of seconds for a distant loiterer, which drives `p_kill` to 1 for *every* pairing.
The diminishing-return discount `(1 − p)^k` then collapses to 1 and stops separating
"cover another drone" from "pile onto this one" — which is the entire job it is there to
do. That degeneracy was observed while building V59, not theorised.

One consequence is worth stating plainly rather than hiding, because it reads as
counter-intuitive: a bomber seconds from release has a *short* window and therefore a
*low* `P(kill before release)`, so it scores below a recce drone the battery can
comfortably catch. That is the formulation being self-consistent, not a bug — maximising
expected value destroyed says shoot what you can still stop, and a bomber past the point
of interception is a lost cause. Whether it is the *right* objective is a separate
question: making `value` reflect imminent harm, rather than only what an airframe carries,
is the natural way to change the answer.

`P(kill | window)` is `air_defence::p_kill_in_window`, which is the same §9.4 pair of laws
V48 and V49 gate — exponential for a gun, geometric for a missile — evaluated forward over
a window rather than sampled, so the two cannot drift apart. `value(air)` is the optional
`value` dial on `AirType`, or a derivation from remaining munitions and whether the
airframe carries a sensor, so an unscored stat block still ranks a loaded bomber above a
spent one.

#### Measured: coordination is about *ammunition*, not kills *(2026-08-05)*

`scenarios/ad_c2.toml` — three SAM batteries against a tight packet of ten drones, 500
seeds, compared **paired**:

| | Downed (of 10) | Rounds left (of 24) | Leakers |
|---|---|---|---|
| No C2 | 9.33 ± 0.04 | 0.82 ± 0.06 | 0.77 ± 0.02 |
| With C2 | 9.92 ± 0.01 | **3.65 ± 0.11** | 0.69 ± 0.02 |
| Paired difference | +0.59 ± 0.04 (t = 15) | **+2.83 ± 0.11 (t = 27)** | −0.08 ± 0.02 (t = −4) |

The kill count barely moves. What moves is the **magazine**: the coordinated defence ends
with four and a half times the reserve, having achieved slightly *more*. Uncoordinated, it
very nearly shot itself dry against a raid it was otherwise winning — and a defence out of
rounds is a defence that loses the next raid.

**Why this scenario uses missiles, and why that is the whole point.** A gun is a Poisson
process, so two batteries on one target simply add their kill rates: `λ + λ`, and nothing
is lost. **Stacking guns is not wasteful.** A missile launch is a discrete round out of a
finite magazine, so three interceptors at a drone one would have killed is two rounds that
will not be there for the next one. Coordination pays exactly where the shot is a
*countable resource* — which is a sharper statement than "coordination is good", and it
falls out of the two engagement models rather than being asserted.

### 11.3 Ground fires and the net

§10.2 let a side coordinate its ground fires for free while §11 made air defence pay for a
post. That asymmetry was reasoned, not arbitrary — a battlegroup does share one fire-control
net, where point-defence batteries genuinely are independent sites — but it was an
*argument*, and an argument is not something you can measure.

`[sim] fires_need_c2` makes it a modelled thing. With it on, `Sim::allocate_side` splits a
side in two and solves **two** problems:

| Shooter | Solver |
|---|---|
| inside a live friendly post's (jammed) radius | the scenario's `allocation` — the side-wide assignment |
| outside it | `Independent` — picks for itself, the pre-Phase-10 rule |

Two separate problems rather than one problem with constraints, deliberately. "Not in the
net" means precisely "does not know what anyone else is doing", so an unnetted shooter must
not be allowed to *avoid* a target because a netted one took it. Solving them together
would leak exactly that information.

Being outside the net costs coordination, not the ability to fight: a loose gun still
engages, on its own judgement. What it loses is shown by V63's three-gun case — it opens on
a target one of the netted guns has already destroyed, and its whole volley leaves no trace
in the log. That wasted volley is what the net buys back.

**Off by default.** Turning it on unconditionally would silently reduce every existing
scenario to `independent`, re-baselining the §10.2 allocation result, V56 and V39 at once,
for a reason invisible in the scenario files. As a dial the cost of losing the net is a
number instead:

```
sweep <scn> --param sim.fires_need_c2 --values false,true --seeds 1000 --metric red_cleared_s
```

### 11.4 Deliberate limitations (v1)

- ~~**The post is not attritable yet.**~~ Lifted by §12: posts and batteries carry
  `element_count`, take §2.3 area damage, and `TargetSpec::Named` resolves across all three
  asset lists. V60.
- ~~**The overkill cap is scoped to a fire-control problem, not to a side.**~~ **Resolved by
  removing the cap** (gate V68).

  `max_shooters_per_target` was a *hard* cap: a target offered `min(elements, cap)` slots,
  so once targets were scarcer than shooters the surplus shooters were assigned nothing and
  fired nothing. Because it was applied once per fire-control problem, a side split by
  `fires_need_c2` got two of them and could put more guns to work than a coordinated side
  was allowed to — so **being split up made a side fight better**, measured on
  `scenarios/fires_c2.toml` at −24.5 s of clear time (t = −10.0), monotone in the net's
  radius.

  §10.2 already prices piling on at $(1-\bar q)^k$. Truncating that as well said "rather
  than overkill, do nothing", which is the wrong trade whenever there is nothing else to
  shoot. The cap is gone; every target now offers a slot to every free shooter and the
  discount decides what the marginal one is worth.

  Re-measured after the change: the coordinated baseline improves from 103.40 s to
  **66.80 s** — that gap is the fire the cap was throwing away — and the inversion is gone.
  Coordination is now worth about **−2.2 s** (t = −2.1), in the direction it should be, and
  netting a third gun adds nothing beyond the second.

  The transferable lesson is not about C2: **a dial whose meaning depends on the scope it is
  applied over will invert when that scope changes.** The cap was coherent while a side was
  one problem and stopped being so the moment a side could be two.
- **The link ignores terrain.** Jamming and latency now bear on it, but a ridge between the
  post and a battery does not. A terrain-aware comms model is the natural next refinement.
- **A post cannot be handed off.** There is no notion of a deputy taking over, so killing
  the only post decoheres the defence permanently rather than for a reorganisation delay.

### 11.5 Validation gates (V59, V62, V63)

| # | Property | Reference |
|---|----------|-----------|
| V59 | C2-coordinated air defence | with one drone nearest to every battery, nearest-first sends them all at it while a C2 post makes them cover one drone each; a scenario with **no** post is unchanged from the pre-C2 engine (the §7.4 identity discipline, so V50–V52 cannot move); a **dead** post coordinates nothing, costing no battery, magazine or envelope — only the coordination; and a post coordinates its **own side only**, so with both sides coordinated each battery still follows its own doctrine and makes the same choice it would make with the enemy's post removed |
| V62 | the link degrades, not only dies | an enemy jammer on the post scales its radius by the EW factor, so the flanking batteries drop out and the defence decoheres with nothing destroyed; a *friendly* jammer does not cut its own net; a **zero-power** jammer runs the whole arithmetic and changes nothing (§7.4); `link_latency_s` delays joining, and a battery not yet in the net commits its channel nearest-first, so a late link cannot undo it |
