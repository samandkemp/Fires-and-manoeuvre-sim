[Index](README.md) · [← §8 Electronic warfare & partial observability](08-ew-and-partial-observability.md) · [§10 The decision layer →](10-the-decision-layer.md)

---

## 9. Air: drones & counter-air

Air assets are a **third class** alongside units and sensors: airframes that fly at a
chosen altitude, heading and speed along a flight path or a transit-then-orbit plan.
Red strike drones bomb Blue ground units; Blue defeats them with an **air-defence**
class whose engagement model - and therefore time-to-kill - varies by type. A drone may
instead (or also) carry a sensor, making it a mobile elevated observer.

The OR content is the **sensor-to-shooter timeline**: air defence can be self-cueing or
forced to rely on external cueing across a comms link with a configurable latency, so
raid leakage becomes a measurable function of cue delay, magazine depth, and engagement
channels. That falls straight out of the §3 sensing model.

Decisions (user, 2026-07-29): air is a new asset class (not a unit or sensor variant);
per-instance altitude with an AGL/AMSL reference; **slant range everywhere**; air
defence may carry an organic sensor with a per-instance self-cue switch and a comms
latency dial; strike targeting is **assigned only** (kill-chain logic is deferred as its
own piece); munitions are dials (`munitions` + `expendable`).

### 9.1 Altitude, actor height, and the slant-range convention

**Altitude is per instance**, with a reference frame, because the two behaviours differ
in exactly the way that matters - whether terrain can mask the airframe:

$$
h(p) = a \quad \text{(AGL)}
\qquad\qquad
h(p) = \max\big(0, a - z(p)\big) \quad \text{(AMSL)}
$$

where `a` = `altitude_m`, the case is chosen by `altitude_ref`, and `z(p)` is the ground
elevation under the airframe.

`h` is precisely the **actor height** of §1.2, so LOS, viewshed, and sensing need no
change: `line_of_sight(terrain, a, h_a, b, h_b)` already takes arbitrary endpoint
heights. An AGL drone hugs the terrain and is never masked by the hill it overflies; an
AMSL drone cruises level and *is* masked by higher ground. The `max(0, ·)` clamp means an
AMSL altitude below the local ground is a drone on the deck - degenerate but well-defined,
never negative.

**Slant range replaces horizontal range** (a correction the air case forces, applied
everywhere for one consistent rule):

$$
r_{\text{slant}}(a, h_a, b, h_b) =
\sqrt{ \lVert b - a \rVert^2 + \big((z(b) + h_b) - (z(a) + h_a)\big)^2 }
$$

used for the detection cutoff and falloff `f(r)` of §3.2 and for both weapon range gates
of §2. On flat ground with equal endpoint heights Δ = 0, so it reduces exactly to the old
horizontal range and the §3/§4 gates are unchanged by construction; on relief it differs
by the endpoint height difference. *(Alternative considered: slant only when an endpoint
is airborne - keeps ground-vs-ground bit-identical, but at the price of two range rules
that disagree by design. Rejected: one convention, documented.)*

**Terrain effects on an airborne target.** Concealment and cover are properties of the
cell a target *stands in*; an airborne target is not in it. So an air target contributes
`concealment = 0` to the §3.2 rate and `cover = 0` to damage. Canopy transmittance `τ` is
*not* waived - it is a property of the sightline, so a low drone seen through a belt of
woods is still attenuated exactly as §1.4 says.

### 9.2 Flight kinematics

State: `pos`, `altitude_m` + `altitude_ref`, `heading_deg`, `speed_m_s`. Pure and
RNG-free - flight is deterministic; all air stochasticity lives in detection and
engagement.

A **flight plan** is a waypoint list plus a terminal, which covers both requested
behaviours with one structure:

```
FlightPlan { waypoints: [Vec2], terminal: Hold | Orbit { radius_m, clockwise } }
```

"Fly this path" is `terminal = Hold`; "go here and orbit at radius R" is a single
waypoint with `terminal = Orbit`. The orbit centre is always the final waypoint.

- **Transit.** Advance `speed·dt` along the polyline (the §6.1 route logic). The desired
  heading is the bearing to the next waypoint; the actual heading turns toward it at up
  to `max_turn_rate_deg_s · dt`. A turn-rate limit implies a minimum turn radius
  `r_min = v / ω_max` (ω in rad/s) - the gate for V47.
- **Orbit.** On reaching the terminal waypoint the airframe captures the circle at its
  nearest point, then integrates the phase directly:
  `θ(t + dt) = θ(t) ± (v/R)·dt`, `pos = c + R·(cos θ, sin θ)`, heading = tangent.
  Integrating the phase rather than steering keeps the radius exact (no drift) and gives
  the closed-form lap time `T = 2πR/v`.
- **Endurance.** `endurance_s > 0` removes the airframe once its time aloft exceeds it.

### 9.3 Strike

A strike drone's aim point is its assigned target - a named unit or a fixed point - or,
if none was assigned, its **final waypoint**. (Autonomous target selection is a kill-chain
problem deferred deliberately; see §9.7.) On closing within `release_range_m` (slant) of
the aim point it releases one munition, which is **exactly the §2.3 indirect round**:
burst point `b = aim + N(0, σ²I)` with `σ = cep_m/1.1774`, Carleton damage
`D(ρ) = exp(−ρ²/2R_L²)`, delivered as `D·(1 − cover)`.

One generalisation of §2.3: an indirect round today damages only the unit it was aimed
at, but a strike on a *point* must damage whatever is near the burst. Damage is therefore
applied to **every live ground unit within `3·R_L` of the burst** (beyond 3 R_L the
Carleton kernel is < 1.2e-4 - a documented cutoff that keeps the sweep O(units)). Each
surviving element rolls independently, as §4.1; near-misses feed §4.3 suppression
unchanged.

`munitions` counts releases; `expendable` decides whether the airframe survives its
attack. Together they span the modern spectrum - a reusable guided-bomb carrier
(`munitions = 2, expendable = false`) and a one-way attack munition
(`munitions = 1, expendable = true`) are the same model with different dials.

### 9.4 Air defence - two engagement models, two closed forms

Two models, because **time-to-kill is distributed differently** in each - differing in
shape, not just in mean, so guns and missiles fail differently against a saturating raid.

**Gun / CIWS - a Poisson kill process.** While the target sits in the envelope, kills
arrive at rate `λ_k`; per tick `p = 1 − e^{−λ_k·dt}`. This is structurally identical to
the §3.2 glimpse model, so it inherits its tick-size invariance and its validation
machinery:

$$
\text{TTK} \sim \mathrm{Exp}(\lambda_k) \qquad \mathbb{E}[\text{TTK}] = \frac{1}{\lambda_k} \qquad P(\text{kill by } t) = 1 - e^{-\lambda_k t}
$$

**Missile - discrete shoot-look-shoot.** A launch takes `t_f = r_slant / missile_speed`
to arrive, then resolves as a Bernoulli trial with single-shot kill probability `p`; a
miss is followed by `t_r` reload before the next launch. Shots-to-kill is Geometric(p),
and the time to the N-th arrival is `N·t_f + (N−1)·t_r`, so

$$
\mathbb{E}[\text{shots}] = \frac{1}{p} \qquad \mathbb{E}[\text{TTK}] = \frac{t_f}{p} + \left(\frac{1}{p} - 1\right) t_r
$$

Note the asymmetry in the *simulation* of this: `resolve_due` charges `t_r` after a **miss**
only, so a battery that kills cleanly may relaunch on the next tick. That reads as the
reload being the *look* in shoot-look-shoot - you only re-engage a target you failed to
kill - and it is self-consistent. It is not what the closed form above assumes: `E[TTK]`,
`shot_opportunities` and therefore the §11.2 allocation payoff all price a `t_f + t_r` cycle
per shot regardless of outcome. The payoff is thus very slightly pessimistic about a
battery's throughput against a stream of targets. Stated rather than reconciled, because the
two answer different questions - the closed form is about killing *one* target, the
simulation about what a battery does next.

*(Alternative considered for the missile: model interception kinematically, with the
missile as a pursuing body and the kill depending on closing geometry. Rejected for v1 -
the interesting OR variable here is delay and magazine, not endgame guidance; `p` and
`t_f` are the dials that carry the behaviour.)*

**Envelope.** An engagement requires the target inside `[min_range_m, max_range_m]`
(slant) *and* `[min_alt_m, max_alt_m]` - the altitude band is what separates a low-tier
CIWS from a high-tier SAM - plus LOS if `requires_los`, a free engagement channel, and
magazine remaining. `channels` (simultaneous engagements) is the saturation lever a raid
plays against.

### 9.5 The cueing timeline

> Which of this section's dials actually decides raid leakage was measured by global
> sensitivity analysis, not argued: see the worked result in
> [EXPERIMENTS.md](../EXPERIMENTS.md#a-worked-result). Cruise speed dominates cue latency.

A battery acts on whichever route to the track reaches it first - its own radar, or the
network:

```
actionable_at = min( own_sensor_seen,                  // organic: no comms hop
                     first_detected + cue_latency_s )  // handed over the net
              + reaction_time_s
```

`own_sensor_seen` is when **this battery's** organic sensor first saw the target, and is
unavailable if it has no sensor, its per-instance `self_cue` switch is off, its radar is not
`emitting`, or it simply has not seen the target yet. Turning `self_cue` off forces the asset
onto the external cueing chain and makes it pay `cue_latency_s` - the comms Tx/Rx lever. The
two switches are distinct: `self_cue` is whose track the battery acts on, `emitting` is
whether its radar is running at all (§12.5).

Taking the **minimum** is what makes this exact rather than approximate. Every airframe
records when each sensor first saw it (`AirState.seen_by`), so a self-cueing battery whose
radar acquires the target *after* someone else's sensor detected it still engages off its
own radar instead of waiting out a comms hop it never needed. *(Keyed
"self-cued" off whoever detected first, which got that case wrong; the per-sensor record
replaced it. Consequence: the air detection loop runs each sensor's glimpse process until
**that sensor** has seen the target, rather than stopping at the first global detection.)*

This yields the phase's headline closed form, and it turns on **what the clock starts
on**: the cueing chain begins at *detection*, not at envelope entry. Let a drone be detected
`D` seconds before it enters the envelope (its **warning lead**) and spend `W` seconds
inside the envelope before reaching its release point. The battery is actionable from
`t_entry − D + L + R`, so the effective engagement window is

$$
W_{\text{eff}} = \max\big(0, W - \max(0, L + R - D)\big)
$$

The delay costs nothing until `L + R` outruns `D`: a cue that has already aged through
the network while the drone was still inbound arrives ready. Consequently the **critical
latency**, beyond which every drone leaks however lethal the battery is, is

$$
L^* = W + D - R
$$

and early warning raises it one second per second - early-warning range and comms latency
trade *directly* against each other. For a gun `P(leak) = exp(−λ_k · W_eff)`; for a
missile with `K = ⌊(W_eff − t_f)/(t_f + t_r)⌋ + 1` shot opportunities (0 if `W_eff < t_f`),
`P(leak) = (1 − p)^K`.

*(The `D = 0` case - acquired exactly as it enters the envelope - gives the simpler
`W_eff = W − L − R` and `L* = W − R`. That special case is what the `air_raid` experiment
isolates in sweep 1a by setting the radar's range equal to the gun's; sweep 1b sweeps `D`
deliberately. The general form above was corrected after the first sweep read zero
leakage everywhere: with a 5 km radar and a 2.5 km envelope the transit time absorbed the
entire cue delay, which is the model behaving correctly and the earlier formula being an
unstated special case.)*

### 9.6 Determinism & the air-off identity

Air adds phases to the loop. They are **appended, and draw zero RNG values when the air
and air-defence lists are empty**, so a drone-free scenario reproduces the pre-air event
log bit-for-bit - the identity discipline §7.4 states in general and EW follows in §8
(V40). The full phase order lives in **§7.2**; the air phases are 2, 4, 6 and 7.

A recce drone's sensor needs no phase of its own: **every sensor lives in one list**, and
a carried one reports the position, height and facing of the airframe carrying it
(`Sim::sensor_view`). For an uncarried sensor that resolves to exactly its own position
and `mount_height_m`, so step 3 is unchanged - same draws, same order - whenever there is
no air. Steps 4, 6 and 7 iterate empty lists in that case and draw nothing at all.

A carried sensor's public `pos`/`facing_deg` are also **written back from the airframe each
tick**, immediately after air movement. The airframe stays the source of truth and
`sensor_view` is still the accessor that knows about altitude, but leaving the public
fields frozen at the placement point made any consumer outside the detection loop - the
app's coverage and belief overlays, the `duel_probe` diagnostic - plot a recce drone's
sensor at its take-off position and ground mount height. `Sim::sensor_active` is the
matching gate: a carried sensor dies with its airframe, so a shot-down drone must drop out
of coverage rasters as well as out of the detection loop.

Ground fires cannot accidentally engage air: target selection iterates the *unit* list, so
the separation is structural rather than a gate that could be forgotten.
`WeaponType.engages_air` (default false) exists as the opt-in seam for a future dual-role
gun; it changes nothing today.

### 9.7 Deliberate limitations

Stated rather than hidden:

- **No autonomous target selection.** Strike targets are assigned (§9.3); a drone will
  not opportunistically attack what its own sensor finds.
- **No air-to-air.** Drones do not engage other drones.
- **Acoustic detection of drones** is the natural modality and remains unimplemented -
  §3.1's `Modality` tag is the seam.
### 9.8 Validation gates (V44-V52)

| # | Property | Reference |
|---|----------|-----------|
| V44 | altitude & masking | an AMSL drone below a ridge crest is masked from a ground sensor while the same drone at AGL is not; visibility is monotone in altitude (extends V8) |
| V45 | slant range | equals `√(horizontal² + Δz²)` exactly; a drone at altitude `A` directly overhead has range `A`, not 0; reduces to horizontal range when Δz = 0 (so V14-V18 stand unchanged) |
| V46 | orbit kinematics | orbiting at radius `R` and speed `v` holds the radius to within ε and closes a lap in `2πR/v` (± one tick) |
| V47 | transit & turn rate | straight-leg travel = `speed·t` (mirrors V37); under a turn-rate limit the achieved turn radius ≥ `v/ω_max` |
| V48 | gun time-to-kill | MC mean TTK = `1/λ_k` and `P(kill by t) = 1 − e^{−λ_k t}` within binomial CI - the §9.4 exponential law, gated as V14/V15 |
| V49 | missile time-to-kill | per-shot kill fraction = `ssk_p` within binomial CI; `E[shots] = 1/p` (geometric); `E[TTK] = t_f/p + (1/p − 1)t_r` |
| V50 | cue latency & leakage | leakage rises monotonically in `cue_latency_s`, matches `exp(−λ_k·W_eff)`, and reaches 1 above the critical latency; warning lead `D` raises `L* = W + D − R` one second per second, and `D = 0` reproduces `W − L − R` |
| V51 | envelope & magazine gating | exactly zero engagements outside the slant-range band, outside the altitude band, without LOS when `requires_los`, without a cue when `self_cue` is off, or with an empty magazine; concurrent engagements never exceed `channels` |
| V52 | air-off identity | no air and no air defence ⇒ event log bit-identical to the pre-air build; with air, same `(scenario, seed)` reproduces exactly |
