[Index](README.md) · [← §11 Command and control](11-command-and-control.md) · [§13 The kill chain: directed targeting →](13-the-kill-chain.md)

---

## 12. SEAD: air defence as a target *(Phase 12)*

§9.7 listed "air-defence sites are not attritable" as a deliberate v1 limitation, and §11
then made it awkward: the model claimed a command post was the thing worth killing first,
while providing no way to kill one from inside the simulation. This closes that.

### 12.1 What changed

**Batteries and posts have elements.** `AirDefenceType.element_count` and
`C2Type.element_count` (both defaulting to 1) give them the same sub-element attrition as
a unit (§4.1), so a near miss degrades a battery rather than only ever destroying or
sparing it. `alive()` is `elements > 0` on both.

**Any ground asset can be named as a target.** `TargetSpec::Named(id)` resolves against
units, then air-defence batteries, then C2 posts. Ids are unique within a scenario, so one
namespace covers all three and SEAD needs no new syntax — `target = { unit = "sam-1" }`
simply works, with `asset` as the clearer alias. (The enum was `TargetSpec::Unit`; the
rename is what the variant now means.)

**Area damage sweeps them.** The §2.3 Carleton kernel is applied to batteries and posts
exactly as to units, with terrain cover, rolled per surviving element. Nothing about the
maths cared which list an asset lived in; only the sweep did.

### 12.2 What death costs

The interesting part is not the destruction but its consequences, and the two differ:

| Asset destroyed | Firepower lost | Second-order effect |
|---|---|---|
| **Battery** | its launchers | **its organic radar goes dark** — an emitter the rest of the network was cueing from (§9.5) |
| **C2 post** | **none at all** | the group it coordinated **decoheres** and reverts to nearest-first (§11) |

A destroyed battery also drops its open engagements, so its channels are not left occupied
by a corpse.

The radar consequence is what makes SEAD worth more than the launchers it removes, and it
falls out of existing structure rather than needing a special case: an organic radar is an
ordinary entry in the sensor list, and `Sim::sensor_active` — which already knew a carried
sensor dies with its airframe — now also knows a radar dies with its battery. Coverage and
belief rasters drop it automatically, because they were already asking that question.

### 12.3 Anti-radiation homing: the radar buys its own accuracy

§12.2 made a battery's radar the thing worth killing. This makes it the thing that makes
killing it *possible*.

A real anti-radiation missile rides the radar's own signal down, so its accuracy is bought
with the target's emissions. Two dials on the weapon say so:

```toml
[weapons.arm]
anti_radiation = true
cep_m          = 5.0     # against a transmitting radar
silent_cep_m   = 400.0   # against a silent one
```

`WeaponType::cep_against(emitting)` is the single place that decides, and for anything
without the flag it returns `cep_m` whatever the emitter is doing — a dumb shell's accuracy
does not depend on what its target is transmitting. So every existing munition is an exact
identity (§7.4), and an ARM with no `silent_cep_m` stated falls back to `cep_m`, meaning
declaring the flag alone changes nothing until the degradation is given a number.

**A dispersion, not a veto.** The munition still arrives; with nothing to home on it flies
to where the emitter was last known to be. "An ARM cannot engage a silent radar at all" is
this with the value set very large — reachable as a scenario's choice, rather than baked in
as the model's opinion. The veto version would also flatter the counter: switching a radar
off would become a free and total defence.

**What counts as an emitter.** Only a *named*, live, `self_cue` battery with a working
organic radar. A command post, a unit or a bare map point radiates nothing an ARM could
ride, so an ARM sent at one is flying blind by definition rather than by omission.

**The trade this poses, and why it is a real one.** `self_cue = false` is the counter, and
it is not free: the battery drops onto the network cueing chain and pays `cue_latency_s` on
every track (§9.5) — the same delay V50 shows deciding whether a SAM gets to fire at all.
So the defender chooses: *survive the missile, or see the raid coming.* Not both.

### 12.4 Ground counter-battery

Air-delivered SEAD was the only kind, because ground fires iterated the unit list. Every
asset class already had elements and already took §2.3 area damage identically, so the only
thing missing was *which lists are searched*. `FireTarget` now names the list, and
`TargetState` gathers the four facts a shell depends on — where, how big, how many left, is
it locatable — so counter-battery arrived by widening a list rather than by writing a second
fires model.

Units come first in `engageable_targets`, batteries and posts are appended. A scenario with
no enemy emplacements therefore produces exactly the list it always did (§7.4).

**How an emplacement is found is the interesting part.** Neither batteries nor posts go
through the §3.2 glimpse loop, so neither has a track, and indirect fire needs one. Inventing
a stochastic acquisition here would have inserted draws into every scenario fielding air
defence and shifted the stream under V50, V51, V59 and V60 for no modelling gain. So
`Sim::emplacement_is_located` asks the question counter-battery acquisition actually asks —
**has it given itself away?**

| Asset | Located when |
|---|---|
| battery | it is transmitting (`self_cue` with a live radar) **or** it has fired |
| post | it is coordinating at least one live battery |

Those are the two real ways a site is fixed: ESM on its emissions, or a counter-battery
track back along its rounds. A command post is found because it is *talking* — the same
argument in a different band. All three are deterministic and draw no randomness.

"Has fired" is read from `AirDefenceState::last_fired_s`, set when an engagement resolves.
It used to be recovered by scanning the whole air-defence event log, inside a test that runs
once per (shooter, target) pair per epoch — so the cost of an epoch grew with how long the
battle had already lasted rather than with its size.

Two things about that flag are **open questions rather than settled model**, and are worth
knowing before a result leans on them:

- **It never expires.** A battery that fired one round at t = 5 s is still located at
  t = 600 s, with no re-acquisition. A *unit* seen once is not: §10.1 lapses its track after
  `track_hold_s`. The two acquisition models therefore disagree about how long knowledge
  lasts, and nothing yet says why.
- **It records a resolution, not a trigger pull.** For a missile battery those coincide. For
  a gun, `resolve_due` logs only a tick that *killed*, so a gun that has been firing steadily
  and hitting nothing has not "fired" for this purpose — which is not what the prose above
  claims.

This joins the two halves of §12.3. Switching a radar off already made an ARM miss; it now
also hides the battery from artillery. **One decision, two consequences** — and the cost
stays what §12.3 said it was, a battery on the network cueing chain paying `cue_latency_s`.

Direct fire is unchanged: line of sight and range, no track (§2.1). So going silent hides a
battery from the guns behind the hill, not from the tank looking at it.

**Value has no derivation across classes.** A unit's is `elements × (1 + threat/threat_max)`
with threat `rof × lethality × reach`. A battery's danger is to *aircraft* and a post has no
firepower at all, so neither has an output measurable on that scale, and a conversion would
be arithmetic dressed as doctrine. Both fall back to 1.0 per element, and a scenario that
wants artillery to prefer the SAM over the tanks says so with `value`. That is what the dial
is for — "kill the radar first" is a judgement, not a derivation.

### 12.5 Deliberate limitations (v1)

- **Targeting is still assigned.** A strike drone attacks what a scenario named; even an
  ARM homes on the emitter it was *sent* at rather than scanning for the nearest
  transmitting radar. Autonomous target selection remains the deferred kill-chain work
  (§9.7).
- **Emissions are binary.** A radar is on or off; there is no intermittent emission, no
  blinking to reduce exposure, and no memory of a position after the emitter goes quiet
  beyond the aim point itself.
- **`self_cue` carries two meanings, and they are not the same thing.** §9.5 introduced it
  as the *cueing-timeline* lever — "this battery is cued from elsewhere and pays
  `cue_latency_s`" — and §12.3 reused it as the *emission* test in `target_is_emitting`.
  A battery with `self_cue = false` therefore counts as silent to an anti-radiation missile
  while its organic radar keeps detecting perfectly well.

  Measured on `scenarios/sead_arm.toml`: the "silent" battery records 1.000 detections with
  first contact at 9.3 s, statistically indistinguishable from the emitting arm's 0.996 at
  10.1 s — so it gets the survivability of EMCON (0.10 vs 0.98 batteries killed) without the
  blindness that should pay for it.

  The genuinely blind case is a battery with no organic sensor at all (`gun_truck`), which
  gets the same protection (−0.877 ± 0.019 batteries killed, t = −46.1) *and* never fires a
  shot. That is the trade §12.3 describes; `self_cue = false` is currently a cheaper version
  of it. Splitting the flag in two — `emitting` for the seeker, `self_cue` for the timeline
  — is the fix, and it moves V64's fixture, so it has not been done unilaterally.

### 12.6 Validation gates (V60, V64, V65)

| # | Property | Reference |
|---|----------|-----------|
| V64 | anti-radiation homing | the same missile lands with `cep_m` against a transmitting radar and `silent_cep_m` against a silent one, the mean miss scaling as the ratio of the two CEPs (`E|miss| = σ√(π/2)`, `σ = CEP/√(2 ln 2)`) and doing correspondingly less damage; a weapon without the flag ignores the emitter entirely and an undeclared `silent_cep_m` falls back to `cep_m`, so both are exact identities; only a named, live, self-cueing battery counts as an emitter |
| V65 | ground counter-battery | a howitzer kills an emitting SAM and the fire log names what it hit; a battery that has neither transmitted nor fired is **not located**, so indirect fire has nothing to aim at; direct fire needs no track, so a silent battery in plain view is still a target; a post is located while it is coordinating and stops being so when it has nothing left to coordinate; a scenario with no enemy emplacements is unchanged (§7.4) |
| V60 | SEAD | a strike drone assigned a named C2 post destroys it **in-simulation** and the defence decoheres with no battery lost; a destroyed battery's organic radar stops emitting; a target id matching nothing yields no aim point, so the new asset lists are additive rather than a replacement (§7.4) |
