[Index](README.md) · [← §3 Sensing & detection](03-sensing.md) · [§5 Movement as DP →](05-movement-as-dp.md)

---

## 4. Suppression & attrition

Decisions (user, 2026-07-29): units are **N sub-elements** attriting one at a time;
suppression is a **discrete Free/Suppressed/Pinned Markov chain** driven by near-miss
volume; fires validated against **Lanchester's square law**.

### 4.1 Units as N elements

A unit type carries `element_count` (default 1). `UnitState` tracks `elements`
(remaining) of `initial_elements`; `strength() = elements / initial` for display,
`alive() = elements > 0`. Attrition removes whole elements:

- **Direct round:** one round that passes `p_hit · p_kill_given_hit · (1 − cover)`
  removes **one** element.
- **Indirect round:** a burst delivers Carleton damage `D` to the unit; each surviving
  element is independently killed with probability `D · (1 − cover)` (a binomial draw) -
  so area fire attrits a group properly, and expected casualties `= elements · D · (1−cover)`.

Fire **volume scales with the shooter's live elements**: a unit fires
`round(rof · epoch/60) · elements` rounds/epoch (each element shoots). This is what makes
the aimed-fire duel obey Lanchester's square law.

### 4.2 Fire gating (revised)

- **Direct fire:** engage the nearest enemy in clear LOS and range - *no detection
  required* (you shoot what you can see).
- **Indirect fire:** engage the nearest **detected** enemy in range (you bombard where
  you have been cued). LOS not required (ballistic arc).

### 4.3 Suppression - the Markov chain

Per-unit state `S ∈ {Free, Suppressed, Pinned}`. Near-misses (rounds landing within
`suppression_radius_m` of the unit that do **not** kill it) push the state up; time
pushes it back down. Per near-miss, with probability `p_suppress` the state steps up one
level (Free→Suppressed→Pinned). Each tick, with rate `recover_per_s` the state steps
down one level (a memoryless recovery timer). Effects:

- **Free:** normal.
- **Suppressed:** outgoing fire effectiveness × `suppressed_fire_factor` (< 1) - degraded
  volume/accuracy; may still move.
- **Pinned:** cannot fire and cannot move (gates the Phase 5 movement layer).

Dials (`scenarios/*.toml`, placeholders): `suppression_radius_m`, `p_suppress`,
`recover_per_s`, `suppressed_fire_factor`.

**What counts as a near-miss differs by weapon class, and the difference is a
simplification worth stating.** Indirect fire samples an impact point and tests it:
a round is a near-miss when `‖burst − target‖ < suppression_radius_m`. **Direct fire does
not.** It resolves a single hit/kill roll and treats *every* miss as a near-miss, whatever
the range, so `suppression_radius_m` does not enter the direct-fire path at all.

At the shipped dials this is very nearly exact - 0.4 mrad at 3 km is σ ≈ 1.2 m, so
essentially every direct-fire miss really does land inside a 35 m radius. It stops being
exact for a high-dispersion or very long-range direct weapon, where suppression would scale
with *rounds fired* rather than with rounds landing close.

It is left as it is deliberately. Sampling an impact point for direct fire would change the
number of RNG draws per round, which re-baselines V24, V30 and V31 - a real cost for an
effect the current dials cannot resolve. Recorded here so the assumption is visible rather
than discovered.

### 4.4 Validation gates (V28-V31)

| # | Property | Reference |
|---|----------|-----------|
| V28 | chain stationary distribution | under a constant near-miss rate `λ_nm` and recovery `μ`, the long-run occupancy of {Free,Suppressed,Pinned} matches the analytic stationary distribution of the birth-death chain within CI |
| V29 | recovery time | with no incoming fire, mean time Pinned→Free = `2/recover_per_s` (two exponential steps), within CI |
| V30 | Lanchester square law | an aimed direct-fire duel (no terrain, no suppression) reproduces `α(A₀²−A²) = β(B₀²−B²)`: many-trial mean force curves match the ODE solution within CI |
| V31 | suppression gates fire | a Pinned unit emits no rounds; a Suppressed unit's expected output = `suppressed_fire_factor` × Free output |
